//! Fixture-driven [`Host`] for tests (also used by other crates' tests).
use super::{Host, Load};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct State {
    now: f64,
    files: HashMap<String, String>,
    commands: HashMap<String, String>,
    slow: HashMap<String, String>,
    ints: HashMap<String, i64>,
    raw: HashMap<String, Vec<u8>>,
    strings: HashMap<String, String>,
    ran: Vec<String>,
    load: Option<Load>,
    root: bool,
}

/// Files, commands ("file arg arg" -> stdout) and sysctls.
pub struct FakeHost {
    freebsd: bool,
    s: Mutex<State>,
}

impl FakeHost {
    pub fn new(freebsd: bool) -> Self {
        Self {
            freebsd,
            s: Mutex::new(State {
                now: 1_000_000.0,
                ..Default::default()
            }),
        }
    }
    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn file(&self, path: &str, text: &str) -> &Self {
        self.st().files.insert(path.into(), text.into());
        self
    }
    pub fn command(&self, line: &str, out: &str) -> &Self {
        self.st().commands.insert(line.into(), out.into());
        self
    }
    pub fn slow(&self, line: &str, out: &str) -> &Self {
        self.st().slow.insert(line.into(), out.into());
        self
    }
    pub fn int(&self, name: &str, v: i64) -> &Self {
        self.st().ints.insert(name.into(), v);
        self
    }
    pub fn raw(&self, name: &str, v: Vec<u8>) -> &Self {
        self.st().raw.insert(name.into(), v);
        self
    }
    pub fn string(&self, name: &str, v: &str) -> &Self {
        self.st().strings.insert(name.into(), v.into());
        self
    }
    pub fn set_load(&self, l: Load) {
        self.st().load = Some(l);
    }
    pub fn set_root(&self, r: bool) {
        self.st().root = r;
    }
    pub fn advance(&self, secs: f64) {
        self.st().now += secs;
    }
    pub fn set_now(&self, secs: f64) {
        self.st().now = secs;
    }
    /// Command lines run so far.
    pub fn ran(&self) -> Vec<String> {
        self.st().ran.clone()
    }
}

impl Host for FakeHost {
    fn is_freebsd(&self) -> bool {
        self.freebsd
    }
    fn now(&self) -> f64 {
        self.st().now
    }
    fn read_text(&self, path: &str) -> Option<String> {
        self.st().files.get(path).cloned()
    }
    fn run(&self, argv: &[&str]) -> Option<String> {
        let key = argv.join(" ");
        let mut s = self.st();
        s.ran.push(key.clone());
        s.commands.get(&key).cloned()
    }
    fn run_slow(&self, argv: &[&str]) -> Option<String> {
        self.st().slow.get(&argv.join(" ")).cloned()
    }
    fn sysctl_i64(&self, name: &str) -> Option<i64> {
        self.st().ints.get(name).copied()
    }
    fn sysctl_bytes(&self, name: &str) -> Option<Vec<u8>> {
        self.st().raw.get(name).cloned()
    }
    fn sysctl_string(&self, name: &str) -> Option<String> {
        self.st().strings.get(name).cloned()
    }
    fn load_average(&self) -> Option<Load> {
        self.st().load
    }
    fn list_dir(&self, dir: &str) -> Vec<String> {
        let prefix = format!("{dir}/");
        let mut v: Vec<String> = self
            .st()
            .files
            .keys()
            .filter_map(|k| {
                k.strip_prefix(&prefix)
                    .map(|r| format!("{prefix}{}", r.split('/').next().unwrap_or("")))
            })
            .collect();
        v.sort();
        v.dedup();
        v
    }
    fn is_root(&self) -> bool {
        self.st().root
    }
}
