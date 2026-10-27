//! One supervisor per job: start/stop on request, restart per `wol_core::jobs::next_start`.
use crate::control::JobTable;
use crate::reaper::Reaper;
use shared::jobs::{JobSpec, JobStatus};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// OS side of a job: start the program (returns pid) and stop a process tree.
pub trait Runner: Send + Sync {
    fn spawn(&self, spec: &JobSpec) -> std::io::Result<i32>;
    fn stop_tree(&self, root: i32);
}

#[derive(Default)]
struct State {
    /// Generation of the current "wanted" request; None = stopped by request.
    wanted: Option<u64>,
    generation: u64,
    pid: Option<i32>,
    starts: u32,
    last_exit: Option<i32>,
    waiting: bool,
}

pub struct Supervisor {
    pub spec: JobSpec,
    state: Mutex<State>,
    wake: Condvar,
}

/// Shared context for every supervisor.
pub struct Ctx {
    pub runner: Arc<dyn Runner>,
    pub reaper: Arc<Reaper>,
    pub stopping: Arc<AtomicBool>,
    pub epoch: Instant,
}

impl Supervisor {
    pub fn new(spec: JobSpec) -> Arc<Self> {
        Arc::new(Self {
            spec,
            state: Mutex::default(),
            wake: Condvar::new(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn pid(&self) -> Option<i32> {
        self.lock().pid
    }

    pub fn status(&self) -> JobStatus {
        let s = self.lock();
        let state = if s.pid.is_some() {
            "running"
        } else if s.waiting {
            "waiting"
        } else {
            "stopped"
        };
        JobStatus {
            label: self.spec.label.clone(),
            state: state.into(),
            pid: s.pid,
            starts: s.starts,
            last_exit: s.last_exit,
        }
    }

    /// None on success, else an error for the control client.
    pub fn start(self: &Arc<Self>, ctx: &Arc<Ctx>) -> Option<String> {
        let generation = {
            let mut s = self.lock();
            if s.wanted.is_some() {
                return Some("already started".into());
            }
            s.generation += 1;
            s.wanted = Some(s.generation);
            s.generation
        };
        let (me, ctx) = (Arc::clone(self), Arc::clone(ctx));
        std::thread::spawn(move || me.run(generation, &ctx));
        None
    }

    pub fn stop(&self, ctx: &Ctx) -> Option<String> {
        let root = {
            let mut s = self.lock();
            if s.wanted.take().is_none() {
                return Some("not started".into());
            }
            s.pid
        };
        self.wake.notify_all();
        if let Some(r) = root {
            ctx.runner.stop_tree(r);
        }
        None
    }

    fn wanted(&self, generation: u64, ctx: &Ctx) -> bool {
        self.lock().wanted == Some(generation) && !ctx.stopping.load(Ordering::Relaxed)
    }

    fn run(&self, generation: u64, ctx: &Ctx) {
        while self.wanted(generation, ctx) {
            let started = ctx.epoch.elapsed().as_secs_f64();
            eprintln!("wold: start {}", self.spec.label);
            match ctx.runner.spawn(&self.spec) {
                Ok(pid) => {
                    let rx = ctx.reaper.register(pid);
                    {
                        let mut s = self.lock();
                        s.pid = Some(pid);
                        s.starts += 1;
                        s.waiting = false;
                    }
                    let code = rx.recv().unwrap_or(-1);
                    let mut s = self.lock();
                    s.pid = None;
                    s.last_exit = Some(code);
                    eprintln!("wold: {} exited {code}", self.spec.label);
                }
                Err(e) => eprintln!("wold: {} failed to start: {e}", self.spec.label),
            }
            let now = ctx.epoch.elapsed().as_secs_f64();
            let Some(next) = wol_core::jobs::next_start(&self.spec, started, now) else {
                break;
            };
            let wait = Duration::from_secs_f64((next - now).max(0.0));
            if wait.is_zero() {
                continue;
            }
            let mut s = self.lock();
            s.waiting = true;
            let deadline = Instant::now() + wait;
            while s.wanted == Some(generation) && Instant::now() < deadline {
                let left = deadline.saturating_duration_since(Instant::now());
                s = self
                    .wake
                    .wait_timeout(s, left)
                    .unwrap_or_else(|p| p.into_inner())
                    .0;
            }
            s.waiting = false;
        }
        let mut s = self.lock();
        s.waiting = false;
        // A run-once job that finished may be started again.
        if s.wanted == Some(generation) {
            s.wanted = None;
        }
    }
}

/// All jobs by label.
pub struct Jobs {
    pub ctx: Arc<Ctx>,
    pub table: BTreeMap<String, Arc<Supervisor>>,
}

impl Jobs {
    pub fn new(ctx: Arc<Ctx>, specs: Vec<JobSpec>) -> Self {
        let table = specs
            .into_iter()
            .map(|s| (s.label.clone(), Supervisor::new(s)))
            .collect();
        Self { ctx, table }
    }

    pub fn start_at_load(&self) {
        for s in self.table.values().filter(|s| s.spec.run_at_load) {
            s.start(&self.ctx);
        }
    }

    pub fn is_job(&self, pid: i32) -> bool {
        self.table.values().any(|s| s.pid() == Some(pid))
    }

    pub fn running(&self) -> Vec<i32> {
        self.table.values().filter_map(|s| s.pid()).collect()
    }
}

impl JobTable for Jobs {
    fn list(&self) -> Vec<JobStatus> {
        self.table.values().map(|s| s.status()).collect()
    }
    fn start(&self, label: &str) -> Option<Option<String>> {
        self.table.get(label).map(|s| s.start(&self.ctx))
    }
    fn stop(&self, label: &str) -> Option<Option<String>> {
        self.table.get(label).map(|s| s.stop(&self.ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicI32;

    /// Spawns nothing: hands out pids and records stops; the test plays the kernel via the reaper.
    #[derive(Default)]
    struct FakeRunner {
        next: AtomicI32,
        stopped: Mutex<Vec<i32>>,
    }

    impl Runner for FakeRunner {
        fn spawn(&self, _: &JobSpec) -> std::io::Result<i32> {
            Ok(1000 + self.next.fetch_add(1, Ordering::SeqCst))
        }
        fn stop_tree(&self, root: i32) {
            self.stopped.lock().unwrap().push(root);
        }
    }

    fn ctx(runner: Arc<FakeRunner>) -> Arc<Ctx> {
        Arc::new(Ctx {
            runner,
            reaper: Arc::default(),
            stopping: Arc::default(),
            epoch: Instant::now(),
        })
    }

    fn spec(json: &str) -> JobSpec {
        JobSpec::parse(json).unwrap()
    }

    fn until(f: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn run_once_job_runs_then_stops() {
        let c = ctx(Arc::default());
        let s = Supervisor::new(spec(r#"{"label":"a","program":["x"]}"#));
        assert_eq!(s.start(&c), None);
        assert_eq!(s.start(&c).as_deref(), Some("already started"));
        until(|| s.pid() == Some(1000));
        assert_eq!(s.status().state, "running");
        c.reaper.exited(1000, 3, true);
        until(|| s.status().state == "stopped");
        let st = s.status();
        assert_eq!((st.starts, st.last_exit), (1, Some(3)));
        until(|| s.start(&c).is_none()); // may be started again
    }

    #[test]
    fn keep_alive_restarts_and_stop_kills_tree() {
        let r = Arc::new(FakeRunner::default());
        let c = ctx(r.clone());
        let s = Supervisor::new(spec(
            r#"{"label":"k","program":["x"],"keepAlive":true,"throttleSeconds":0}"#,
        ));
        s.start(&c);
        until(|| s.pid() == Some(1000));
        c.reaper.exited(1000, 1, true);
        until(|| s.pid() == Some(1001));
        assert_eq!(s.status().starts, 2);
        assert_eq!(s.stop(&c), None);
        assert_eq!(*r.stopped.lock().unwrap(), vec![1001]);
        assert_eq!(s.stop(&c).as_deref(), Some("not started"));
        c.reaper.exited(1001, 143, true);
        until(|| s.status().state == "stopped");
        assert_eq!(s.status().starts, 2, "no restart after stop");
    }

    #[test]
    fn throttled_job_waits() {
        let c = ctx(Arc::default());
        let s = Supervisor::new(spec(
            r#"{"label":"t","program":["x"],"keepAlive":true,"throttleSeconds":60}"#,
        ));
        s.start(&c);
        until(|| s.pid().is_some());
        c.reaper.exited(1000, 0, true);
        until(|| s.status().state == "waiting");
        s.stop(&c); // wakes the wait
        until(|| s.status().state == "stopped");
    }

    #[test]
    fn table_routes_and_reports_unknown() {
        let c = ctx(Arc::default());
        let jobs = Jobs::new(
            c,
            vec![spec(r#"{"label":"a","program":["x"],"runAtLoad":false}"#)],
        );
        jobs.start_at_load();
        assert_eq!(jobs.list()[0].state, "stopped");
        assert_eq!(JobTable::stop(&jobs, "nope"), None);
        assert_eq!(JobTable::start(&jobs, "a"), Some(None));
        until(|| jobs.is_job(1000));
    }
}
