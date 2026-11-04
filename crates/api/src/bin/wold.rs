// SPDX-License-Identifier: GPL-2.0-or-later
//! wold [jobs-dir]: launchd-style supervisor. Composition root: load jobs, wire services, run.
use api::{bus::Bus, jobfiles, memsaver, os, qos, reaper, server, supervisor};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, atomic::AtomicBool};

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| shared::jobs::JOBS_DIR.into());
    os::block_signals(); // before any thread: signals are taken synchronously below
    // R2C-8: copies of package installs a previous wold left behind (crash, stop mid-sheet).
    let swept = api::packages::sweep_staging(Path::new(api::packages::STAGING));
    if swept > 0 {
        eprintln!("wold: removed {swept} stale staged package copies");
    }
    if !reaper::acquire() {
        eprintln!("wold: could not become reaper");
    }
    let (specs, errors) = jobfiles::load(Path::new(&dir));
    for e in errors {
        eprintln!("wold: skipped {e}");
    }
    // A job a previous wold left running (crash, `systemctl restart wold`) is adopted by its
    // supervisor when it starts (`Runner::adopt`), not stopped: the login window and the session
    // outlive the supervisor (QA-SYS-2).
    let pid_dir = PathBuf::from(jobfiles::PID_DIR);
    settle_stale(&pid_dir, &specs);
    let ctx = Arc::new(supervisor::Ctx {
        runner: Arc::new(os::OsRunner {
            qos: Box::new(qos::Sched),
            pid_dir,
        }),
        reaper: reaper::global().clone(),
        stopping: Arc::new(AtomicBool::new(false)),
        epoch: std::time::Instant::now(),
    });
    let jobs = Arc::new(supervisor::Jobs::new(ctx.clone(), specs));
    let bus: Arc<Bus<std::os::unix::net::UnixStream>> = Arc::default();
    jobs.start_at_load();

    let saver = Arc::new(memsaver::Service::new(shared::memsaver::STATUS));
    spawn_memsaver(saver.clone());
    spawn_qos(saver.clone());
    // The compositor, shell and shader precompilers start after wold: re-check every few seconds
    // (idempotent).
    start_thread("session-tick", || {
        loop {
            qos::session_tick(Path::new("/proc"));
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
    {
        let (jobs, bus) = (jobs.clone(), bus.clone());
        let grants = Arc::new(api::portal::ConsentFile(wol_core::consent::FILE.into()));
        let state = Arc::new(api::portal::State {
            background: Box::new(api::services::AppJobs::new(ctx.clone())),
            ..Default::default()
        });
        let listener =
            server::bind(Path::new(shared::jobs::SOCKET), 0o666).expect("bind control socket");
        eprintln!("wold: control listening on {}", shared::jobs::SOCKET);
        start_thread("control-accept", move || {
            // Concurrent connections (each a thread; holds last as long as the app): 64 per uid,
            // 512 in total, so one account cannot exhaust wold's threads (audit A9/A10).
            let quota = shared::bounded::Quota::new(64, 512);
            for conn in listener.incoming() {
                let Ok(conn) = conn else {
                    // EMFILE and friends: wait instead of spinning at 100 % CPU (audit S4-26).
                    std::thread::sleep(ACCEPT_BACKOFF);
                    continue;
                };
                // Identity now, at accept, not after the request was read (audit A2).
                let peer = api::peer::PeerIdentity::capture(&conn);
                let Some(slot) = peer.as_ref().and_then(|p| quota.admit(p.uid)) else {
                    eprintln!("wold: refused a connection over its cap");
                    refuse_busy(conn);
                    continue;
                };
                let (jobs, bus, grants, state) =
                    (jobs.clone(), bus.clone(), grants.clone(), state.clone());
                spawn_conn("control", move || {
                    server::serve(conn, peer, &*jobs, &bus, &*grants, &state);
                    drop(slot);
                });
            }
        });
    }

    loop {
        match os::wait_signal() {
            libc::SIGCHLD => {
                ctx.reaper.reap_all(|p| jobs.is_job(p));
            }
            // S4-2: a hang-up (a terminal that became ours and closed) or a broken pipe is never
            // a reason to take the whole session down.
            libc::SIGHUP | libc::SIGPIPE => {}
            _ => {
                ctx.stopping
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                saver.shutdown();
                // The stop that starts a `systemctl restart wold` leaves the jobs running for
                // the next wold to adopt (QA-SYS-2); any other stop ends them.
                if os::restarting() {
                    eprintln!("wold: restarting; jobs keep running");
                } else {
                    os::stop_all(&jobs.running());
                }
                std::process::exit(0);
            }
        }
    }
}

/// What the previous wold left behind, said once at start: a job's own program still running is
/// adopted (by `start_at_load`), a copy of a program replaced since is stopped, and a pid file that
/// points at something else (a recycled pid) is dropped without signalling that process.
fn settle_stale(pid_dir: &Path, specs: &[shared::jobs::JobSpec]) {
    for spec in specs {
        let Some((pid, verdict)) = os::stale_of(pid_dir, spec, Path::new("/proc")) else {
            continue;
        };
        match verdict {
            os::Stale::Adopt => eprintln!("wold: adopting {} (pid {pid})", spec.label),
            os::Stale::Outdated => {
                eprintln!("wold: stopping outdated {} (pid {pid})", spec.label);
                os::stop_tree(pid);
            }
            os::Stale::Foreign => {
                eprintln!(
                    "wold: {}: pid {pid} is another program; left alone",
                    spec.label
                );
            }
        }
        if verdict != os::Stale::Adopt
            && let Some(f) = jobfiles::pid_file(pid_dir, &spec.label)
        {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// What a client over its connection cap hears (QA-SHADOW-41: it used to be dropped before the
/// hello and report "wold unavailable" while wold ran).
const BUSY: &str = "wold is busy with other requests from this account. Try again in a moment.";

/// At most this many busy replies at once; past it the connection is just closed.
const BUSY_REPLIES: usize = 8;

/// Greet the connection, read what it sends (so closing does not reset the reply), answer
/// [`BUSY`], on a short-lived thread of its own.
fn refuse_busy(conn: std::os::unix::net::UnixStream) {
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ANSWERING: AtomicUsize = AtomicUsize::new(0);
    if ANSWERING.fetch_add(1, Ordering::SeqCst) >= BUSY_REPLIES {
        ANSWERING.fetch_sub(1, Ordering::SeqCst);
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("wold-busy".into())
        .spawn(move || {
            let _ = shared::bounded::limit_socket(&conn, server::CLIENT_TIMEOUT);
            if shared::jobs::greet(&conn).is_ok() {
                let mut line = [0u8; 1];
                let mut got = 0;
                while got < shared::portal::MAX_LINE
                    && (&conn).read(&mut line).is_ok_and(|n| n == 1)
                    && line[0] != b'\n'
                {
                    got += 1;
                }
                let _ = writeln!(&conn, "{}", shared::jobs::Reply::of(Some(BUSY.into())));
            }
            ANSWERING.fetch_sub(1, Ordering::SeqCst);
        });
    if spawned.is_err() {
        ANSWERING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Pause after a failed accept (out of descriptors): the error repeats until a connection ends.
const ACCEPT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);

/// The realtime sockets (QoS, Memory Saver): one wol-realtime per account, so one connection per
/// uid, and room for every seat (audit S4-1: four in total let one account starve the others).
const REALTIME_PER_UID: usize = 1;
const REALTIME_TOTAL: usize = 64;

/// Frontmost boost plus Memory Saver: the app coming to the front stops giving memory back.
struct Boost(Arc<memsaver::Service>);

impl qos::Applier for Boost {
    fn apply(&self, pid: i32, class: shared::qos::QosClass) -> bool {
        if class == shared::qos::QosClass::UserInteractive {
            self.0.focus(pid);
        }
        qos::Sched.apply(pid, class)
    }
    fn gpu_foreground(&self, pid: i32) {
        qos::Sched.gpu_foreground(pid)
    }
}

/// Memory Saver (ADR 0022): snapshots from wol-realtime only, one ticker thread.
fn spawn_memsaver(saver: Arc<memsaver::Service>) {
    let Some(listener) = bind_or_report("Memory Saver", shared::memsaver::SOCKET) else {
        return;
    };
    let allowed = shared::sys::canonical(qos::REALTIME_EXE);
    {
        let saver = saver.clone();
        start_thread("memsaver-tick", move || saver.run_ticker());
    }
    start_thread("memsaver-accept", move || {
        let quota = shared::bounded::Quota::new(REALTIME_PER_UID, REALTIME_TOTAL);
        for conn in listener.incoming() {
            let Ok(conn) = conn else {
                std::thread::sleep(ACCEPT_BACKOFF);
                continue;
            };
            let id = api::peer::PeerIdentity::capture(&conn);
            let ok = id.as_ref().is_some_and(|p| {
                p.still_same()
                    && p.exe()
                        .is_some_and(|e| qos::peer_allowed(e.as_bytes(), &allowed))
            });
            let (true, Some(id)) = (ok, id) else {
                eprintln!("wold: memsaver refused peer");
                continue;
            };
            let Some(slot) = quota.admit(id.uid) else {
                eprintln!("wold: memsaver refused a connection over its cap");
                continue;
            };
            let saver = saver.clone();
            spawn_conn("memsaver", move || {
                // ADR 0033: greet first, as on the control socket.
                if shared::jobs::greet(&conn).is_ok() {
                    saver.serve(conn, id.peer());
                }
                drop(slot);
            });
        }
    });
}

fn spawn_qos(saver: Arc<memsaver::Service>) {
    let Some(listener) = bind_or_report("QoS", shared::qos::SOCKET) else {
        return;
    };
    let allowed = shared::sys::canonical(qos::REALTIME_EXE);
    // S4-1: one frontmost app per account, not one for everybody.
    let foregrounds = Arc::new(Mutex::new(qos::Foregrounds::default()));
    let n = qos::hint_defaults(Path::new(qos::HINTS));
    eprintln!(
        "wold: qos listening on {} ({n} kernel hints)",
        shared::qos::SOCKET
    );
    start_thread("qos-accept", move || {
        let quota = shared::bounded::Quota::new(REALTIME_PER_UID, REALTIME_TOTAL);
        for conn in listener.incoming() {
            let Ok(conn) = conn else {
                std::thread::sleep(ACCEPT_BACKOFF);
                continue;
            };
            let id = api::peer::PeerIdentity::capture(&conn);
            let admitted = id
                .as_ref()
                .filter(|p| {
                    p.still_same()
                        && p.exe()
                            .is_some_and(|e| qos::peer_allowed(e.as_bytes(), &allowed))
                })
                .and_then(|p| Some((p.uid, quota.admit(p.uid)?)));
            let Some((uid, slot)) = admitted else {
                eprintln!("wold: qos refused peer");
                continue;
            };
            let foregrounds = foregrounds.clone();
            let boost = Boost(saver.clone());
            spawn_conn("qos", move || {
                if shared::jobs::greet(&conn).is_ok() {
                    // S4-1: only pids of the peer's own account (real uid in /proc/<pid>/status).
                    let owner = |pid: i32| memsaver::procs::owner(Path::new("/proc"), pid);
                    qos::serve(conn, uid, &owner, &foregrounds, &boost);
                }
                drop(slot);
            });
        }
    });
}

/// Where wold lists the features it started without (a socket it could not bind), one per line,
/// for `wol doctor` and whoever reads the journal. Gone with the next boot (/run).
const DEGRADED: &str = "/run/wold.degraded";

/// The line for a socket that could not be bound. The `<3>` prefix is journald's "error"
/// priority, so `journalctl -p err -u wold` shows it (QA-SHADOW-19: it used to be a plain stderr
/// line and the feature stayed off without anybody being told).
fn bind_failure_line(what: &str, path: &str, err: &std::io::Error) -> String {
    format!("<3>wold: {what} is OFF: cannot bind {path}: {err}")
}

/// Bind `path` (world connectable) for `what`; when that fails say so loudly (journal at error
/// priority, [`DEGRADED`]) and carry on without the feature.
fn bind_or_report(what: &str, path: &str) -> Option<std::os::unix::net::UnixListener> {
    match server::bind(Path::new(path), 0o666) {
        Ok(l) => Some(l),
        Err(e) => {
            report_degraded(Path::new(DEGRADED), what, path, &e);
            None
        }
    }
}

fn report_degraded(file: &Path, what: &str, path: &str, err: &std::io::Error) {
    eprintln!("{}", bind_failure_line(what, path, err));
    use std::io::Write;
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .and_then(|mut f| writeln!(f, "{what}: {path}: {err}"));
}

/// A thread wold cannot work without (an accept loop, a ticker): if it does not start, wold
/// exits so systemd starts it again, instead of living on with a dead socket that looks healthy
/// (QA-SHADOW-40: `std::thread::spawn` panics in the thread that called it).
fn start_thread(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(work) {
        eprintln!("<2>wold: cannot start the {name} thread: {e}");
        std::process::exit(1);
    }
}

/// One connection's thread. A refusal (EAGAIN: TasksMax, RLIMIT_NPROC, memory) drops that one
/// connection, says so, and makes the accept loop wait ([`CONN_BACKOFF`]) instead of spinning;
/// wold itself stays up and serves the next one. Returns whether the thread started.
fn spawn_conn(what: &str, work: impl FnOnce() + Send + 'static) -> bool {
    try_spawn_conn(
        what,
        work,
        |w| std::thread::Builder::new().spawn(w).map(drop),
        std::thread::sleep,
    )
}

/// [`spawn_conn`] with the thread start and the wait given, so a test can refuse the start.
fn try_spawn_conn<W>(
    what: &str,
    work: W,
    spawn: impl FnOnce(W) -> std::io::Result<()>,
    wait: impl FnOnce(std::time::Duration),
) -> bool {
    match spawn(work) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("<3>wold: {what}: cannot start a connection thread, dropping it: {e}");
            wait(CONN_BACKOFF);
            false
        }
    }
}

/// Pause after a refused connection thread: the shortage lasts until something ends.
const CONN_BACKOFF: std::time::Duration = std::time::Duration::from_millis(500);

#[cfg(test)]
mod tests {
    use super::*;

    /// QA-SHADOW-19: a socket that cannot be bound is an error-priority journal line naming the
    /// feature and the path, and a line in the degraded list.
    #[test]
    fn socket_bind_err_is_loud() {
        let d = std::env::temp_dir().join(format!("wold-bind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let no_dir = d.join("missing").join("wol-qos.sock");
        let e = server::bind(&no_dir, 0o666).unwrap_err();
        let line = bind_failure_line("QoS", &no_dir.to_string_lossy(), &e);
        assert!(line.starts_with("<3>wold: QoS is OFF"), "{line}");
        assert!(line.contains("wol-qos.sock"), "{line}");
        let list = d.join("wold.degraded");
        report_degraded(&list, "QoS", "/run/wol-qos.sock", &e);
        report_degraded(&list, "Memory Saver", "/run/wol-memsaver.sock", &e);
        let text = std::fs::read_to_string(&list).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.starts_with("QoS: /run/wol-qos.sock: "), "{text}");
        std::fs::remove_dir_all(&d).ok();
    }

    /// QA-SHADOW-40: a refused connection thread is dropped with a wait and no panic; the next
    /// connection is served. Before, `std::thread::spawn` panicked in the accept thread, which
    /// ended while wold's signal loop kept the unit "active".
    #[test]
    fn spawn_conn_err_backs_off() {
        let waited = std::cell::Cell::new(None);
        let ran = std::cell::Cell::new(false);
        let refused = try_spawn_conn(
            "control",
            || ran.set(true),
            |_| Err(std::io::Error::from_raw_os_error(libc::EAGAIN)),
            |d| waited.set(Some(d)),
        );
        assert!(!refused, "reported as not started");
        assert!(!ran.get());
        assert_eq!(waited.get(), Some(CONN_BACKOFF));
        waited.set(None);
        let started = try_spawn_conn("control", || {}, |_| Ok(()), |d| waited.set(Some(d)));
        assert!(started);
        assert_eq!(waited.get(), None, "no wait when the thread started");
    }

    /// QA-SHADOW-40, the class: nothing in this binary starts a thread with `std::thread::spawn`
    /// (it panics when the system refuses); [`start_thread`] and [`spawn_conn`] are the doors.
    #[test]
    fn no_bare_thread_spawn_in_wold() {
        let src = include_str!("wold.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        let bare = ["std::thread::", "spawn("].concat();
        assert!(
            !code.lines().any(|l| l.contains(&bare)),
            "use start_thread / spawn_conn"
        );
    }

    /// QA-SYS-2: what the previous wold left is settled at start: a recycled pid (another
    /// program) loses its pid file and is not signalled; a copy of a program that was replaced on
    /// disk (an update) is stopped; the job's own program is kept for adoption.
    #[test]
    fn settle_stale_adopts_ours_stops_outdated_and_spares_foreign() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("wold-settle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let sleep = std::fs::canonicalize("/usr/bin/sleep").unwrap();
        // A private copy of sleep, started and then deleted: `/proc/<pid>/exe (deleted)`.
        let copy = d.join("job-prog");
        std::fs::copy(&sleep, &copy).unwrap();
        std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut outdated = std::process::Command::new(&copy).arg("30").spawn().unwrap();
        std::fs::remove_file(&copy).unwrap();
        let mut ours = std::process::Command::new(&sleep)
            .arg("30")
            .spawn()
            .unwrap();
        let mut foreign = std::process::Command::new(&sleep)
            .arg("30")
            .spawn()
            .unwrap();
        let job = |label: &str, program: &str| {
            shared::jobs::JobSpec::parse(&format!(
                r#"{{"label":"{label}","program":["{program}"]}}"#
            ))
            .unwrap()
        };
        let pidfile = |label: &str, pid: u32| {
            std::fs::write(d.join(format!("{label}.pid")), pid.to_string()).unwrap()
        };
        pidfile("ours", ours.id());
        pidfile("outdated", outdated.id());
        pidfile("foreign", foreign.id());
        let specs = [
            job("ours", &sleep.to_string_lossy()),
            job("outdated", &copy.to_string_lossy()),
            job("foreign", "/usr/bin/true"),
        ];
        settle_stale(&d, &specs);
        assert!(d.join("ours.pid").exists(), "kept for adoption");
        assert!(!d.join("foreign.pid").exists());
        assert!(!d.join("outdated.pid").exists());
        assert!(
            foreign.try_wait().unwrap().is_none() && ours.try_wait().unwrap().is_none(),
            "neither is signalled"
        );
        assert!(
            outdated.wait().unwrap().code().is_none(),
            "the outdated copy was stopped by a signal"
        );
        for c in [&mut ours, &mut foreign] {
            c.kill().unwrap();
            c.wait().unwrap();
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// QA-SHADOW-41: a connection over its cap is greeted and told it is busy, not dropped.
    #[test]
    fn over_cap_connection_is_told_busy() {
        use std::io::{BufRead, Write};
        let (server_end, client) = std::os::unix::net::UnixStream::pair().unwrap();
        refuse_busy(server_end);
        let _ = shared::bounded::limit_socket(&client, std::time::Duration::from_secs(5));
        shared::jobs::answer_hello(&client).unwrap();
        (&client).write_all(b"list\n").unwrap();
        let mut reply = String::new();
        std::io::BufReader::new(&client)
            .read_line(&mut reply)
            .unwrap();
        assert!(reply.contains("busy"), "{reply}");
    }
}
