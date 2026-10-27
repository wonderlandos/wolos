//! System API: what the shell and `wol sys` read about the machine (see archicture/modules/system-api.md).
//! Per topic: pure parser (fixture-tested) + a read through [`Host`]. Snapshots are cached per topic.
pub mod fake;
pub mod info;
pub mod proc;
pub mod stats;

use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Load {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

/// OS access for system services. [`PosixHost`] is the real one; [`fake::FakeHost`] for tests.
pub trait Host: Send + Sync {
    fn is_freebsd(&self) -> bool;
    /// Unix seconds.
    fn now(&self) -> f64;
    fn read_text(&self, path: &str) -> Option<String>;
    /// First bytes of a file into `buf`; returns length. Real host does not allocate.
    fn read_head(&self, path: &str, buf: &mut [u8]) -> Option<usize> {
        let t = self.read_text(path)?;
        let n = t.len().min(buf.len());
        buf[..n].copy_from_slice(&t.as_bytes()[..n]);
        Some(n)
    }
    /// Stdout of argv, None when missing/failing/timed out (2 s).
    fn run(&self, argv: &[&str]) -> Option<String>;
    /// Like `run` for slow commands (minutes); call off the UI thread.
    fn run_slow(&self, argv: &[&str]) -> Option<String> {
        self.run(argv)
    }
    fn sysctl_i64(&self, name: &str) -> Option<i64>;
    fn sysctl_bytes(&self, name: &str) -> Option<Vec<u8>>;
    /// sysctl into a caller buffer; returns length. Real host does not allocate.
    fn sysctl_into(&self, name: &str, buf: &mut [u8]) -> Option<usize> {
        let b = self.sysctl_bytes(name)?;
        let n = b.len().min(buf.len());
        buf[..n].copy_from_slice(&b[..n]);
        Some(n)
    }
    fn sysctl_string(&self, name: &str) -> Option<String>;
    fn load_average(&self) -> Option<Load>;
    /// Full paths of entries of `dir`, sorted; empty if missing.
    fn list_dir(&self, dir: &str) -> Vec<String>;
    fn is_root(&self) -> bool {
        false
    }
}

pub const RUN_TIMEOUT: Duration = Duration::from_secs(2);

/// Real host: files, sysctl(3) on FreeBSD, child processes with a timeout.
#[derive(Default, Debug, Clone, Copy)]
pub struct PosixHost;

/// Run argv, capture stdout, kill after `timeout`. Never a shell.
pub fn run_with_timeout(argv: &[&str], timeout: Duration) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let (file, args) = argv.split_first()?;
    let mut child = Command::new(file)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out.read_to_end(&mut s);
        s
    });
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    }?;
    let bytes = reader.join().ok()?;
    status
        .success()
        .then(|| String::from_utf8_lossy(&bytes).into_owned())
}

impl Host for PosixHost {
    fn is_freebsd(&self) -> bool {
        cfg!(target_os = "freebsd")
    }
    fn now(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
    }
    fn read_text(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
    fn read_head(&self, path: &str, buf: &mut [u8]) -> Option<usize> {
        use std::io::Read;
        std::fs::File::open(path).ok()?.read(buf).ok()
    }
    fn run(&self, argv: &[&str]) -> Option<String> {
        run_with_timeout(argv, RUN_TIMEOUT)
    }
    fn run_slow(&self, argv: &[&str]) -> Option<String> {
        run_with_timeout(argv, Duration::from_secs(120))
    }
    fn sysctl_i64(&self, name: &str) -> Option<i64> {
        let b = sysctl::read(name)?;
        match b.len() {
            4 => Some(i64::from(i32::from_ne_bytes(b[..4].try_into().ok()?))),
            8 => Some(i64::from_ne_bytes(b[..8].try_into().ok()?)),
            _ => None,
        }
    }
    fn sysctl_bytes(&self, name: &str) -> Option<Vec<u8>> {
        sysctl::read(name)
    }
    fn sysctl_into(&self, name: &str, buf: &mut [u8]) -> Option<usize> {
        sysctl::read_into(name, buf)
    }
    fn sysctl_string(&self, name: &str) -> Option<String> {
        let b = sysctl::read(name)?;
        let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
        Some(String::from_utf8_lossy(&b[..end]).into_owned())
    }
    fn load_average(&self) -> Option<Load> {
        let mut l = [0f64; 3];
        // SAFETY: getloadavg writes at most 3 doubles into l.
        let n = unsafe { libc::getloadavg(l.as_mut_ptr(), 3) };
        (n == 3).then(|| Load {
            one: l[0],
            five: l[1],
            fifteen: l[2],
        })
    }
    fn list_dir(&self, dir: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
    fn is_root(&self) -> bool {
        // SAFETY: geteuid has no preconditions.
        unsafe { libc::geteuid() == 0 }
    }
}

/// sysctlbyname(3). FreeBSD only; None elsewhere.
pub mod sysctl {
    /// Into a caller buffer; returns bytes written. Does not allocate.
    #[cfg(target_os = "freebsd")]
    pub fn read_into(name: &str, buf: &mut [u8]) -> Option<usize> {
        let mut cname = [0u8; 128];
        if name.len() >= cname.len() {
            return None;
        }
        cname[..name.len()].copy_from_slice(name.as_bytes());
        let mut len: libc::size_t = buf.len();
        // SAFETY: cname is NUL-terminated; sysctlbyname writes at most len bytes into buf.
        let r = unsafe {
            libc::sysctlbyname(
                cname.as_ptr().cast(),
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null(),
                0,
            )
        };
        (r == 0).then_some(len)
    }

    #[cfg(not(target_os = "freebsd"))]
    pub fn read_into(_name: &str, _buf: &mut [u8]) -> Option<usize> {
        None
    }

    pub fn read(name: &str) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; 4096];
        let n = read_into(name, &mut buf)?;
        buf.truncate(n);
        Some(buf)
    }
}

/// A value reloaded at most once per `ttl`. Thread-safe. A failing load keeps the last value.
pub struct Cached<T> {
    load: Box<dyn Fn() -> Option<T> + Send + Sync>,
    ttl: f64,
    clock: Arc<dyn Host>,
    state: Mutex<(T, Option<f64>)>,
}

impl<T: Clone> Cached<T> {
    pub fn new(
        clock: Arc<dyn Host>,
        ttl_secs: f64,
        fallback: T,
        load: impl Fn() -> Option<T> + Send + Sync + 'static,
    ) -> Self {
        Self {
            load: Box::new(load),
            ttl: ttl_secs,
            clock,
            state: Mutex::new((fallback, None)),
        }
    }

    pub fn is_fresh(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.1.is_some_and(|t| self.clock.now() - t < self.ttl)
    }

    pub fn get(&self) -> T {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = self.clock.now();
        if s.1.is_some_and(|t| now - t < self.ttl) {
            return s.0.clone();
        }
        if let Some(v) = (self.load)() {
            s.0 = v;
        }
        s.1 = Some(now);
        s.0.clone()
    }

    pub fn invalidate(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).1 = None;
    }
}

/// Topics `wol sys <topic>` knows.
pub const TOPICS: [&str; 7] = [
    "about",
    "stats",
    "disks",
    "temperatures",
    "gpu",
    "hw",
    "updates",
];

/// Entry point: cached snapshots per topic. Snapshots never fail; failures yield last/default values.
pub struct SystemApi {
    pub host: Arc<dyn Host>,
    about: Cached<info::SystemInfo>,
    stats: Cached<stats::SystemStats>,
    disks: Cached<Vec<stats::DiskUsage>>,
    temps: Cached<Vec<stats::Temperature>>,
    gpus: Cached<Vec<String>>,
    hw: Cached<shared::hardware::HardwareSnapshot>,
    updates: Cached<Option<Vec<String>>>,
}

impl SystemApi {
    pub fn new(host: Arc<dyn Host>) -> Self {
        let h = host.clone();
        let src = Mutex::new(stats::StatsSource::new(host.clone()));
        Self {
            about: Cached::new(host.clone(), 1e12, info::SystemInfo::unknown(), {
                let h = h.clone();
                move || Some(info::SystemInfo::read(&*h))
            }),
            stats: Cached::new(
                host.clone(),
                0.9,
                stats::SystemStats::default(),
                move || Some(src.lock().unwrap_or_else(|e| e.into_inner()).read()),
            ),
            disks: Cached::new(host.clone(), 30.0, vec![], {
                let h = h.clone();
                move || Some(stats::DiskUsage::read(&*h))
            }),
            temps: Cached::new(host.clone(), 5.0, vec![], {
                let h = h.clone();
                move || Some(stats::Temperature::read(&*h))
            }),
            gpus: Cached::new(host.clone(), 1e12, vec![], {
                let h = h.clone();
                move || Some(stats::gpu::read(&*h))
            }),
            hw: Cached::new(host.clone(), 30.0, Default::default(), {
                let h = h.clone();
                move || Some(crate::hardware::read(&*h))
            }),
            updates: Cached::new(host.clone(), 3600.0, None, {
                let h = h.clone();
                move || {
                    Some(crate::update::parse_pending(
                        h.run_slow(&crate::update::PENDING_ARGV).as_deref(),
                    ))
                }
            }),
            host,
        }
    }

    pub fn default_host() -> Self {
        Self::new(Arc::new(PosixHost))
    }

    pub fn about(&self) -> info::SystemInfo {
        self.about.get()
    }
    pub fn stats(&self) -> stats::SystemStats {
        self.stats.get()
    }
    pub fn disks(&self) -> Vec<stats::DiskUsage> {
        self.disks.get()
    }
    pub fn temperatures(&self) -> Vec<stats::Temperature> {
        self.temps.get()
    }
    pub fn gpus(&self) -> Vec<String> {
        self.gpus.get()
    }
    /// Devices with driver/firmware state (`wol hw list`).
    pub fn hardware(&self) -> shared::hardware::HardwareSnapshot {
        self.hw.get()
    }
    /// Pending pkg upgrades (None = unknown). Forks pkg at most once an hour.
    pub fn pending_updates(&self) -> Option<Vec<String>> {
        self.updates.get()
    }
    pub fn invalidate_updates(&self) {
        self.updates.invalidate();
    }

    /// JSON snapshot of one topic; None for unknown topics.
    pub fn topic_json(&self, topic: &str) -> Option<serde_json::Value> {
        let v = |x: serde_json::Result<serde_json::Value>| x.unwrap_or(serde_json::Value::Null);
        Some(match topic {
            "about" => v(serde_json::to_value(self.about())),
            "stats" => v(serde_json::to_value(self.stats())),
            "disks" => v(serde_json::to_value(self.disks())),
            "temperatures" => v(serde_json::to_value(self.temperatures())),
            "gpu" => v(serde_json::to_value(self.gpus())),
            "hw" => v(serde_json::to_value(self.hardware())),
            "updates" => v(serde_json::to_value(self.pending_updates())),
            _ => return None,
        })
    }
}

/// "key=value" / "key: value" lines (first wins).
pub(crate) fn pairs(s: &str, sep: char) -> std::collections::HashMap<&str, &str> {
    let mut d = std::collections::HashMap::new();
    for l in s.lines() {
        if let Some(i) = l.find(sep)
            && i > 0
        {
            d.entry(l[..i].trim()).or_insert(l[i + 1..].trim());
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::fake::FakeHost;
    use super::*;

    #[test]
    fn cached_respects_ttl_fallback_and_invalidate() {
        let h = Arc::new(FakeHost::new(false));
        let n = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let n2 = n.clone();
        let c = Cached::new(h.clone(), 10.0, 0u32, move || {
            let v = n2.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            (v != 2).then_some(v)
        });
        assert_eq!(c.get(), 1);
        assert!(c.is_fresh());
        assert_eq!(c.get(), 1);
        h.advance(11.0);
        assert_eq!(c.get(), 1, "failed load keeps last value");
        c.invalidate();
        assert_eq!(c.get(), 3);
    }

    #[test]
    fn updates_are_cached_and_invalidated() {
        let h = Arc::new(FakeHost::new(true));
        h.slow("pkg version -U -vRL=", "a-1 <   needs updating\n");
        let api = SystemApi::new(h.clone());
        assert_eq!(api.pending_updates(), Some(vec!["a".into()]));
        h.slow("pkg version -U -vRL=", "");
        assert_eq!(api.pending_updates().map(|v| v.len()), Some(1));
        api.invalidate_updates();
        assert_eq!(api.pending_updates().map(|v| v.len()), Some(0));
    }

    #[test]
    fn topics_have_json_and_unknown_is_none() {
        let api = SystemApi::new(Arc::new(FakeHost::new(false)));
        for t in TOPICS {
            assert!(api.topic_json(t).is_some(), "{t}");
        }
        assert!(api.topic_json("nope").is_none());
    }

    #[test]
    fn hw_topic_is_the_hardware_snapshot() {
        // `wol sys hw` = `wol hw list --json`: the same catalogue-resolved devices.
        assert!(TOPICS.contains(&"hw"));
        let api = SystemApi::new(Arc::new(FakeHost::new(false)));
        assert_eq!(
            api.topic_json("hw"),
            Some(serde_json::json!({"devices": []}))
        );
    }

    #[test]
    fn posix_run_times_out_and_captures() {
        assert_eq!(
            run_with_timeout(&["echo", "hi"], RUN_TIMEOUT).as_deref(),
            Some("hi\n")
        );
        assert_eq!(run_with_timeout(&["false"], RUN_TIMEOUT), None);
        assert_eq!(run_with_timeout(&["/does/not/exist"], RUN_TIMEOUT), None);
        let t = std::time::Instant::now();
        assert_eq!(
            run_with_timeout(&["sleep", "5"], Duration::from_millis(100)),
            None
        );
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
