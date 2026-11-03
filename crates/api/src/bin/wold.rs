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
    let pid_dir = PathBuf::from(jobfiles::PID_DIR);
    for s in &specs {
        os::stop_stale(&pid_dir, &s.label);
    }
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
    std::thread::spawn(|| {
        loop {
            qos::protect_session(Path::new("/proc"));
            qos::tag_precompile(Path::new("/proc"), &qos::Sched);
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
    {
        let (jobs, bus) = (jobs.clone(), bus.clone());
        let grants = Arc::new(api::portal::Remembering {
            grants: api::portal::ConsentFile(wol_core::consent::FILE.into()),
            dir: wol_core::consent::ASKED_DIR.into(),
            log: |m| eprintln!("wold: {m}"),
        });
        let state = Arc::new(api::portal::State {
            background: Box::new(api::services::AppJobs::new(ctx.clone())),
            ..Default::default()
        });
        let listener =
            server::bind(Path::new(shared::jobs::SOCKET), 0o666).expect("bind control socket");
        eprintln!("wold: control listening on {}", shared::jobs::SOCKET);
        std::thread::spawn(move || {
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
                    continue;
                };
                let (jobs, bus, grants, state) =
                    (jobs.clone(), bus.clone(), grants.clone(), state.clone());
                std::thread::spawn(move || {
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
                os::stop_all(&jobs.running());
                std::process::exit(0);
            }
        }
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
    let listener = match server::bind(Path::new(shared::memsaver::SOCKET), 0o666) {
        Ok(l) => l,
        Err(e) => return eprintln!("wold: memory saver disabled: {e}"),
    };
    let allowed = shared::sys::canonical(qos::REALTIME_EXE);
    {
        let saver = saver.clone();
        std::thread::spawn(move || saver.run_ticker());
    }
    std::thread::spawn(move || {
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
            std::thread::spawn(move || {
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
    let listener = match server::bind(Path::new(shared::qos::SOCKET), 0o666) {
        Ok(l) => l,
        Err(e) => return eprintln!("wold: qos disabled: {e}"),
    };
    let allowed = shared::sys::canonical(qos::REALTIME_EXE);
    // S4-1: one frontmost app per account, not one for everybody.
    let foregrounds = Arc::new(Mutex::new(qos::Foregrounds::default()));
    let n = qos::hint_defaults(Path::new(qos::HINTS));
    eprintln!(
        "wold: qos listening on {} ({n} kernel hints)",
        shared::qos::SOCKET
    );
    std::thread::spawn(move || {
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
            std::thread::spawn(move || {
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
