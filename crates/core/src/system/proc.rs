//! Process table as struct-of-arrays from FreeBSD `kern.proc.proc` (no ps fork), refreshed in place.
//! Steady-state sampling allocates nothing: arrays grow only with the process count, names are cached
//! per (pid, start time), users per uid. CPU% = runtime delta / wall delta per process (100 = one core).
//! Sequential on purpose: IaraOS measured a fork/join of table || totals slower (totals cost ~7 us).
use std::collections::HashMap;

/// FreeBSD 15 amd64 `struct kinfo_proc` layout (sys/user.h). ki_structsize is checked per record.
pub mod table {
    pub const KINFO_SIZE: usize = 1088;
    pub const COMM_OFFSET: usize = 447;
    pub const COMM_LEN: usize = 20;

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Row {
        pub pid: i32,
        pub ppid: i32,
        pub uid: i32,
        pub stat: u8,
        pub rss_pages: i64,
        pub vsize: i64,
        pub runtime_us: i64,
        pub start_us: i64,
        pub nice: i8,
        pub threads: i32,
    }

    fn i32_at(r: &[u8], o: usize) -> i32 {
        i32::from_le_bytes([r[o], r[o + 1], r[o + 2], r[o + 3]])
    }
    fn i64_at(r: &[u8], o: usize) -> i64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&r[o..o + 8]);
        i64::from_le_bytes(b)
    }

    /// Records in `t`, or None when length or any ki_structsize does not match this layout.
    pub fn count(t: &[u8]) -> Option<usize> {
        if !t.len().is_multiple_of(KINFO_SIZE) {
            return None;
        }
        let n = t.len() / KINFO_SIZE;
        (0..n)
            .all(|i| i32_at(t, i * KINFO_SIZE) == KINFO_SIZE as i32)
            .then_some(n)
    }

    pub fn row(t: &[u8], i: usize) -> Row {
        let r = &t[i * KINFO_SIZE..(i + 1) * KINFO_SIZE];
        Row {
            pid: i32_at(r, 72),
            ppid: i32_at(r, 76),
            uid: i32_at(r, 168),
            stat: r[388],
            rss_pages: i64_at(r, 264),
            vsize: i64_at(r, 256),
            runtime_us: i64_at(r, 328),
            start_us: i64_at(r, 336) * 1_000_000 + i64_at(r, 344),
            nice: r[389] as i8,
            threads: i32_at(r, 596),
        }
    }

    /// ki_comm bytes (no NUL).
    pub fn comm(t: &[u8], i: usize) -> &[u8] {
        let c = &t[i * KINFO_SIZE + COMM_OFFSET..i * KINFO_SIZE + COMM_OFFSET + COMM_LEN];
        let end = c.iter().position(|b| *b == 0).unwrap_or(c.len());
        &c[..end]
    }

    /// Encode a row (tests and fakes).
    pub fn write(dst: &mut [u8], r: &Row, comm: &str, structsize: i32) {
        dst[..4].copy_from_slice(&structsize.to_le_bytes());
        dst[72..76].copy_from_slice(&r.pid.to_le_bytes());
        dst[76..80].copy_from_slice(&r.ppid.to_le_bytes());
        dst[168..172].copy_from_slice(&r.uid.to_le_bytes());
        dst[256..264].copy_from_slice(&r.vsize.to_le_bytes());
        dst[264..272].copy_from_slice(&r.rss_pages.to_le_bytes());
        dst[328..336].copy_from_slice(&r.runtime_us.to_le_bytes());
        dst[336..344].copy_from_slice(&(r.start_us / 1_000_000).to_le_bytes());
        dst[344..352].copy_from_slice(&(r.start_us % 1_000_000).to_le_bytes());
        dst[388] = r.stat;
        dst[389] = r.nice as u8;
        dst[596..600].copy_from_slice(&r.threads.to_le_bytes());
        let c = comm.as_bytes();
        let n = c.len().min(COMM_LEN - 1);
        dst[COMM_OFFSET..COMM_OFFSET + n].copy_from_slice(&c[..n]);
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
    /// Raw kinfo_proc table into `buf` (grow only when too small); returns length.
    fn procs(&mut self, buf: &mut Vec<u8>) -> Option<usize>;
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
    table: Vec<u8>,
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

    /// FreeBSD ki_stat: SIDL 1, SRUN 2, SSLEEP 3, SSTOP 4, SZOMB 5, SWAIT 6, SLOCK 7.
    pub fn state(&self, i: usize) -> ProcessState {
        match self.stat[i] {
            2 => ProcessState::Running,
            3 => ProcessState::Sleeping,
            4 => ProcessState::Stopped,
            5 => ProcessState::Zombie,
            6 => ProcessState::Idle,
            7 => ProcessState::Waiting,
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
        let Some(n) = len.and_then(|l| table::count(&self.table[..l])) else {
            self.count = 0;
            return false;
        };
        self.grow(n);
        let wall = now - self.prev_now;
        let page = self.src.page_size();
        self.cur.clear();
        for i in 0..n {
            let r = table::row(&self.table, i);
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
            let comm = table::comm(&self.table, i);
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

/// Product source: kern.proc.proc, kern.cp_time, vm.stats page counts. FreeBSD only.
#[cfg(target_os = "freebsd")]
pub struct FreeBsdProcSource {
    page: i64,
    passwd: Option<HashMap<i32, String>>,
    start: std::time::Instant,
}

#[cfg(target_os = "freebsd")]
impl Default for FreeBsdProcSource {
    fn default() -> Self {
        let mut b = [0u8; 8];
        let page = super::sysctl::read_into("hw.pagesize", &mut b).map_or(4096, |n| match n {
            4 => i64::from(i32::from_ne_bytes([b[0], b[1], b[2], b[3]])),
            _ => i64::from_ne_bytes(b),
        });
        Self {
            page,
            passwd: None,
            start: std::time::Instant::now(),
        }
    }
}

#[cfg(target_os = "freebsd")]
impl ProcSource for FreeBsdProcSource {
    fn procs(&mut self, buf: &mut Vec<u8>) -> Option<usize> {
        let mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PROC];
        for _ in 0..4 {
            if !buf.is_empty() {
                let mut len: libc::size_t = buf.len();
                // SAFETY: sysctl writes at most len bytes into buf.
                let r = unsafe {
                    libc::sysctl(
                        mib.as_ptr(),
                        3,
                        buf.as_mut_ptr().cast(),
                        &mut len,
                        std::ptr::null(),
                        0,
                    )
                };
                if r == 0 {
                    return Some(len);
                }
                if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOMEM) {
                    return None;
                }
            }
            let mut need: libc::size_t = 0;
            // SAFETY: size query with a null buffer.
            let r = unsafe {
                libc::sysctl(
                    mib.as_ptr(),
                    3,
                    std::ptr::null_mut(),
                    &mut need,
                    std::ptr::null(),
                    0,
                )
            };
            if r != 0 || need == 0 {
                return None;
            }
            buf.resize(need + need / 4 + table::KINFO_SIZE, 0);
        }
        None
    }
    fn now_us(&self) -> i64 {
        self.start.elapsed().as_micros() as i64
    }
    fn page_size(&self) -> i64 {
        self.page
    }
    fn totals(&mut self, t: &mut SystemTotals) {
        use super::sysctl::read_into;
        let mut b = [0u8; 40];
        if read_into("kern.cp_time", &mut b) == Some(40) {
            let v =
                |i: usize| i64::from_ne_bytes(b[i * 8..i * 8 + 8].try_into().unwrap_or_default());
            t.cpu_total = (0..5).map(v).sum();
            t.cpu_busy = t.cpu_total - v(4);
        }
        let i = |n: &str| {
            let mut b = [0u8; 8];
            read_into(n, &mut b).map_or(0, |n| {
                if n == 4 {
                    i64::from(u32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
                } else {
                    i64::from_ne_bytes(b)
                }
            })
        };
        t.mem_total = i("vm.stats.vm.v_page_count") * self.page;
        t.mem_used = t.mem_total
            - (i("vm.stats.vm.v_free_count") + i("vm.stats.vm.v_inactive_count")) * self.page;
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

/// Fake kinfo_proc table for tests and non-FreeBSD hosts.
#[derive(Default)]
pub struct FakeProcSource {
    pub table: Vec<u8>,
    pub now: i64,
    pub sys: SystemTotals,
    pub user_lookups: usize,
    pub broken: bool,
}

/// (pid, runtime_us, start_sec, comm, uid, rss_pages, stat)
pub type FakeProc<'a> = (i32, i64, i64, &'a str, i32, i64, u8);

impl FakeProcSource {
    pub fn set(&mut self, rows: &[FakeProc]) {
        self.table.clear();
        self.table.resize(rows.len() * table::KINFO_SIZE, 0);
        let size = if self.broken {
            1000
        } else {
            table::KINFO_SIZE as i32
        };
        for (i, &(pid, runtime_us, start, comm, uid, rss_pages, stat)) in rows.iter().enumerate() {
            let r = table::Row {
                pid,
                runtime_us,
                start_us: start * 1_000_000,
                uid,
                rss_pages,
                stat,
                ..Default::default()
            };
            table::write(
                &mut self.table[i * table::KINFO_SIZE..(i + 1) * table::KINFO_SIZE],
                &r,
                comm,
                size,
            );
        }
    }
}

impl ProcSource for FakeProcSource {
    fn procs(&mut self, buf: &mut Vec<u8>) -> Option<usize> {
        if buf.len() < self.table.len() {
            buf.resize(self.table.len(), 0);
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
        (pid, rt, 100, "sh", 0, 1, 3)
    }
    fn mk() -> ProcessSampler<FakeProcSource> {
        ProcessSampler::new(FakeProcSource::default())
    }
    fn idx(s: &ProcessSampler<FakeProcSource>, pid: i32) -> usize {
        s.pid[..s.count()].iter().position(|x| *x == pid).unwrap()
    }

    #[test]
    fn first_sample_decodes_rows_with_zero_cpu() {
        let mut s = mk();
        s.source().set(&[
            (1, 5_000_000, 100, "init", 0, 3, 2),
            (42, 0, 100, "fish", 1002, 1, 3),
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
        s.source().set(&[(7, 9_000_000, 100, "old", 0, 1, 3)]);
        s.source().now = 1_000_000;
        s.sample();
        s.source().set(&[(7, 100_000, 200, "new", 0, 1, 3)]);
        s.source().now = 2_000_000;
        s.sample();
        assert_eq!((s.cpu_percent[0], s.name[0].as_str()), (0.0, "new"));
        s.source().set(&[(7, 100_000, 200, "vim", 0, 1, 3)]);
        s.sample();
        assert_eq!(s.name[0], "vim");
    }

    #[test]
    fn wrong_layout_fails_closed() {
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
            (1, 0, 100, "b", 0, 5, 3),
            (2, 0, 100, "c", 0, 9, 3),
            (3, 0, 100, "a", 0, 1, 3),
        ]);
        s.sample();
        let mut ix = vec![0; 3];
        s.sort(&mut ix, Column::Memory, true);
        assert_eq!(ix.iter().map(|i| s.pid[*i]).collect::<Vec<_>>(), [2, 1, 3]);
        s.sort(&mut ix, Column::Name, false);
        assert_eq!(ix.iter().map(|i| s.pid[*i]).collect::<Vec<_>>(), [3, 1, 2]);
    }
}
