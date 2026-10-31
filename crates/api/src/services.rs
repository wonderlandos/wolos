// SPDX-License-Identifier: GPL-2.0-or-later
//! An app's background jobs (ADR 0020): wold supervises them like its own, but as the account that
//! asked. The program is always the caller's executable as the kernel saw it, so a service can do
//! nothing its app could not; it gets a fixed argument naming the service and a clean environment.
use crate::portal::Caller;
use crate::supervisor::{Ctx, Supervisor};
use serde_json::{Value, json};
use shared::jobs::JobSpec;
use shared::portal::{MAX_SERVICES, SERVICE_FLAG};
use shared::qos::QosClass;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Seconds between restarts of a service that keeps exiting.
const THROTTLE: u32 = 10;

/// What wold does with services; real in wold, absent in tests of the portal.
pub trait Background: Send + Sync {
    fn start(&self, c: &Caller, name: &str) -> Result<(), &'static str>;
    /// False when this app runs no such service.
    fn stop(&self, c: &Caller, name: &str) -> bool;
    fn status(&self, c: &Caller, name: &str) -> Value;
}

pub struct NoBackground;

impl Background for NoBackground {
    fn start(&self, _: &Caller, _: &str) -> Result<(), &'static str> {
        Err("Background services are not available.")
    }
    fn stop(&self, _: &Caller, _: &str) -> bool {
        false
    }
    fn status(&self, _: &Caller, _: &str) -> Value {
        json!({ "state": "stopped" })
    }
}

/// (account, executable, service).
type Key = (u32, String, String);

pub struct AppJobs {
    ctx: Arc<Ctx>,
    table: Mutex<HashMap<Key, Arc<Supervisor>>>,
}

fn key(c: &Caller, name: &str) -> Key {
    (c.uid, c.app.clone(), name.to_owned())
}

fn spec(c: &Caller, name: &str) -> JobSpec {
    let app = wol_core::package::sha256_hex(c.app.as_bytes());
    JobSpec {
        label: format!("app-{}-{}-{name}", c.uid, &app[..16]),
        program: vec![c.app.clone(), format!("{SERVICE_FLAG}{name}")],
        keep_alive: true,
        throttle_seconds: THROTTLE,
        run_at_load: false,
        qos: Some(QosClass::Background.name().to_owned()),
        user: Some(c.uid),
    }
}

impl AppJobs {
    pub fn new(ctx: Arc<Ctx>) -> Self {
        Self {
            ctx,
            table: Mutex::default(),
        }
    }

    fn table(&self) -> std::sync::MutexGuard<'_, HashMap<Key, Arc<Supervisor>>> {
        self.table.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Background for AppJobs {
    fn start(&self, c: &Caller, name: &str) -> Result<(), &'static str> {
        if c.uid == 0 {
            return Err("Services run for an account, not for the system.");
        }
        let mut t = self.table();
        let k = key(c, name);
        if t.contains_key(&k) {
            return Ok(());
        }
        if t.keys().filter(|k| k.0 == c.uid).count() >= MAX_SERVICES {
            return Err("This account runs too many services.");
        }
        let s = Supervisor::new(spec(c, name));
        match s.start(&self.ctx) {
            None => {
                t.insert(k, s);
                Ok(())
            }
            Some(_) => Err("That service could not start."),
        }
    }

    fn stop(&self, c: &Caller, name: &str) -> bool {
        let s = self.table().remove(&key(c, name));
        s.is_some_and(|s| {
            s.stop(&self.ctx);
            true
        })
    }

    fn status(&self, c: &Caller, name: &str) -> Value {
        let Some(s) = self.table().get(&key(c, name)).map(|s| s.status()) else {
            return json!({ "state": "stopped" });
        };
        let mut v = json!({ "state": s.state, "starts": s.starts });
        if let Some(code) = s.last_exit {
            v["last_exit"] = json!(code);
        }
        v
    }
}

/// An account as a job runs as it.
#[derive(Debug, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub gid: u32,
    pub home: String,
}

pub fn identity(passwd: &str, uid: u32) -> Option<Identity> {
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 6 && f[2].parse() == Ok(uid)).then_some(())?;
        Some(Identity {
            name: f[0].to_owned(),
            gid: f[3].parse().ok()?,
            home: f[5].to_owned(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reaper::Reaper;
    use crate::supervisor::Runner;
    use std::sync::atomic::{AtomicBool, AtomicI32};

    #[derive(Default)]
    struct Fake {
        spawned: Mutex<Vec<JobSpec>>,
        stopped: Mutex<Vec<i32>>,
        last: AtomicI32,
    }

    impl Runner for Fake {
        fn spawn(&self, s: &JobSpec) -> std::io::Result<i32> {
            self.spawned.lock().unwrap().push(s.clone());
            Ok(1000 + self.last.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
        }
        fn stop_tree(&self, root: i32) {
            self.stopped.lock().unwrap().push(root);
        }
    }

    fn jobs() -> (AppJobs, Arc<Fake>) {
        let runner = Arc::new(Fake::default());
        let ctx = Arc::new(Ctx {
            runner: runner.clone(),
            reaper: Arc::new(Reaper::default()),
            stopping: Arc::new(AtomicBool::new(false)),
            epoch: std::time::Instant::now(),
        });
        (AppJobs::new(ctx), runner)
    }

    fn who(uid: u32, app: &str) -> Caller {
        Caller {
            uid,
            pid: 7,
            app: app.into(),
        }
    }

    fn until(f: impl Fn() -> bool) {
        for _ in 0..500 {
            if f() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("timed out");
    }

    #[test]
    fn a_service_is_the_callers_own_program_as_the_caller() {
        let (j, r) = jobs();
        let a = who(1000, "/usr/bin/notes");
        j.start(&a, "sync").unwrap();
        j.start(&a, "sync").unwrap();
        until(|| !r.spawned.lock().unwrap().is_empty());
        until(|| j.status(&a, "sync")["state"] == "running");
        let s = r.spawned.lock().unwrap();
        assert_eq!(s.len(), 1, "started once");
        assert_eq!(s[0].program, ["/usr/bin/notes", "--wol-service=sync"]);
        assert_eq!(s[0].user, Some(1000));
        assert!(s[0].keep_alive);
        assert_eq!(s[0].qos.as_deref(), Some("background"));
    }

    #[test]
    fn only_the_app_that_started_a_service_sees_or_stops_it() {
        let (j, r) = jobs();
        let a = who(1000, "/usr/bin/notes");
        j.start(&a, "sync").unwrap();
        until(|| j.status(&a, "sync")["state"] == "running");
        for other in [who(1000, "/usr/bin/mail"), who(1001, "/usr/bin/notes")] {
            assert!(!j.stop(&other, "sync"));
            assert_eq!(j.status(&other, "sync")["state"], "stopped");
        }
        assert!(j.stop(&a, "sync"));
        assert_eq!(*r.stopped.lock().unwrap(), [1000]);
        assert!(!j.stop(&a, "sync"));
        assert_eq!(j.status(&a, "sync")["state"], "stopped");
    }

    #[test]
    fn root_runs_none_and_an_account_runs_few() {
        let (j, _) = jobs();
        assert!(j.start(&who(0, "/usr/bin/notes"), "sync").is_err());
        let a = who(1000, "/usr/bin/notes");
        for n in 0..MAX_SERVICES {
            j.start(&a, &format!("s{n}")).unwrap();
        }
        assert!(j.start(&a, "more").is_err());
        assert!(j.start(&who(1000, "/usr/bin/mail"), "more").is_err());
        j.start(&who(1001, "/usr/bin/notes"), "more").unwrap();
    }

    #[test]
    fn an_account_comes_from_its_uid() {
        let t = "root:x:0:0::/root:/bin/bash\nalice:x:1000:100:A:/home/alice:/bin/zsh\nbad:x\n";
        let i = identity(t, 1000).unwrap();
        assert_eq!((i.name.as_str(), i.gid), ("alice", 100));
        assert_eq!(i.home, "/home/alice");
        assert_eq!(identity(t, 1), None);
    }
}
