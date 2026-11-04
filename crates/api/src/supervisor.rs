// SPDX-License-Identifier: GPL-2.0-or-later
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
    /// The pid of this job's program a previous wold left running, if any: the supervisor takes
    /// it over instead of starting a second copy (QA-SYS-2).
    fn adopt(&self, _spec: &JobSpec) -> Option<i32> {
        None
    }
    /// An adopted pid is nobody's child here, so nothing reaps it for us: ask whether it runs.
    fn alive(&self, _pid: i32) -> bool {
        false
    }
}

/// How often an adopted process is looked at.
const ADOPTED_POLL: Duration = Duration::from_millis(250);

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
        // A thread the system refuses (EAGAIN: TasksMax, RLIMIT_NPROC) is an error for the client,
        // not a panic in the control connection that asked (QA-SHADOW-40).
        let spawned = std::thread::Builder::new()
            .name(format!("job-{}", self.spec.label))
            .spawn(move || me.run(generation, &ctx));
        if let Err(e) = spawned {
            self.lock().wanted = None;
            return Some(format!("could not start the job's thread: {e}"));
        }
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

    /// The first time round, a process of this job that the previous wold left running is taken
    /// over: tracked, restarted when it ends, stopped on request, as one wold started itself.
    fn run_adopted(&self, pid: i32, ctx: &Ctx) {
        {
            let mut s = self.lock();
            s.pid = Some(pid);
            s.starts += 1;
            s.waiting = false;
        }
        while ctx.runner.alive(pid) {
            std::thread::sleep(ADOPTED_POLL);
        }
        let mut s = self.lock();
        s.pid = None;
        s.last_exit = Some(-1); // nobody reaped it for us: the status is unknown
    }

    fn run(&self, generation: u64, ctx: &Ctx) {
        let mut adoptable = true;
        while self.wanted(generation, ctx) {
            let started = ctx.epoch.elapsed().as_secs_f64();
            let adopted = std::mem::take(&mut adoptable)
                .then(|| ctx.runner.adopt(&self.spec))
                .flatten();
            if let Some(pid) = adopted {
                self.run_adopted(pid, ctx);
            } else {
                eprintln!("wold: start {}", self.spec.label);
                self.run_spawned(ctx);
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

    fn run_spawned(&self, ctx: &Ctx) {
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

    /// Jobs marked run-at-load start, and so does any other job the previous wold left running
    /// (its supervisor takes the process over).
    pub fn start_at_load(&self) {
        for s in self
            .table
            .values()
            .filter(|s| s.spec.run_at_load || self.ctx.runner.adopt(&s.spec).is_some())
        {
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
        /// A process the previous wold left for the first job that asks.
        left_over: Mutex<Option<i32>>,
        /// The pids "alive" for an adopted process.
        running: Mutex<Vec<i32>>,
    }

    impl Runner for FakeRunner {
        fn spawn(&self, _: &JobSpec) -> std::io::Result<i32> {
            Ok(1000 + self.next.fetch_add(1, Ordering::SeqCst))
        }
        fn stop_tree(&self, root: i32) {
            self.stopped.lock().unwrap().push(root);
        }
        fn adopt(&self, _: &JobSpec) -> Option<i32> {
            *self.left_over.lock().unwrap() // idempotent, as the real one is
        }
        fn alive(&self, pid: i32) -> bool {
            self.running.lock().unwrap().contains(&pid)
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

    /// QA-SYS-2: a job the previous wold left running is taken over, not started twice and not
    /// stopped: it is tracked as running, a keep-alive job restarts when it ends, and a stop
    /// reaches its tree.
    #[test]
    fn adopts_alive_stale_job_and_restarts_it_when_it_ends() {
        let r = Arc::new(FakeRunner::default());
        *r.left_over.lock().unwrap() = Some(777);
        r.running.lock().unwrap().push(777);
        let c = ctx(r.clone());
        let s = Supervisor::new(spec(
            r#"{"label":"login","program":["x"],"keepAlive":true,"throttleSeconds":0}"#,
        ));
        s.start(&c);
        until(|| s.pid() == Some(777));
        assert_eq!(s.status().state, "running");
        assert_eq!(r.next.load(Ordering::SeqCst), 0, "nothing was spawned");
        assert!(r.stopped.lock().unwrap().is_empty(), "nothing was stopped");
        r.running.lock().unwrap().clear(); // it ends
        until(|| s.pid() == Some(1000)); // keep-alive: a fresh copy of our own
        assert_eq!(s.status().last_exit, Some(-1));
        assert_eq!(s.status().starts, 2);
        assert_eq!(s.stop(&c), None);
        assert_eq!(*r.stopped.lock().unwrap(), vec![1000]);
    }

    /// A job not marked run-at-load starts at load anyway when the previous wold left it running.
    #[test]
    fn a_left_over_job_starts_at_load_even_when_not_run_at_load() {
        let r = Arc::new(FakeRunner::default());
        *r.left_over.lock().unwrap() = Some(55);
        r.running.lock().unwrap().push(55);
        let c = ctx(r);
        let jobs = Jobs::new(
            c,
            vec![spec(r#"{"label":"a","program":["x"],"runAtLoad":false}"#)],
        );
        jobs.start_at_load();
        until(|| jobs.is_job(55));
    }

    /// The real runner: the pid file left by a previous wold names a live process whose
    /// executable is the job's program -> adopted; a recycled pid (another program) is neither
    /// adopted nor signalled and its file goes; a program replaced on disk is stopped.
    #[test]
    fn adopts_alive_stale_job_with_matching_exe() {
        use crate::os::{OsRunner, Stale, classify_stale};
        let d = std::env::temp_dir().join(format!("wold-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let sleep = std::fs::canonicalize("/usr/bin/sleep")
            .or_else(|_| std::fs::canonicalize("/bin/sleep"))
            .unwrap();
        let mut child = std::process::Command::new(&sleep)
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let runner = OsRunner {
            qos: Box::new(crate::qos::Sched),
            pid_dir: d.clone(),
        };
        let job = |label: &str, program: &str| {
            spec(&format!(r#"{{"label":"{label}","program":["{program}"]}}"#))
        };
        let write_pid =
            |label: &str| std::fs::write(d.join(format!("{label}.pid")), pid.to_string()).unwrap();

        write_pid("ours");
        let ours = job("ours", &sleep.to_string_lossy());
        assert_eq!(runner.adopt(&ours), Some(pid));
        assert!(d.join("ours.pid").exists(), "kept for the next wold");

        write_pid("recycled");
        let other = job("recycled", "/usr/bin/true");
        assert_eq!(
            runner.adopt(&other),
            None,
            "another program is never adopted"
        );
        assert!(crate::os::alive(pid), "and never signalled");

        // The supervisor tracks it as the job's process.
        let c = Arc::new(Ctx {
            runner: Arc::new(runner),
            reaper: Arc::default(),
            stopping: Arc::default(),
            epoch: Instant::now(),
        });
        let sup = Supervisor::new(ours);
        sup.start(&c);
        until(|| sup.pid() == Some(pid));
        assert_eq!(sup.status().state, "running");
        child.kill().unwrap();
        child.wait().unwrap();
        until(|| sup.status().state == "stopped");
        assert_eq!(sup.status().last_exit, Some(-1));

        // The pure classification the runner uses.
        let p = sleep.to_string_lossy();
        assert_eq!(classify_stale(&p, &p), Stale::Adopt);
        assert_eq!(
            classify_stale(&p, &format!("{p} (deleted)")),
            Stale::Outdated
        );
        assert_eq!(classify_stale(&p, "/usr/bin/true"), Stale::Foreign);
        let _ = std::fs::remove_dir_all(&d);
    }
}
