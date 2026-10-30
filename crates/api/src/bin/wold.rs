//! wold [jobs-dir]: launchd-style supervisor. Composition root: load jobs, wire services, run.
use api::{bus::Bus, jobfiles, os, qos, reaper, server, supervisor};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, atomic::AtomicBool};

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| shared::jobs::JOBS_DIR.into());
    os::block_signals(); // before any thread: signals are taken synchronously below
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
        reaper: Arc::default(),
        stopping: Arc::new(AtomicBool::new(false)),
        epoch: std::time::Instant::now(),
    });
    let jobs = Arc::new(supervisor::Jobs::new(ctx.clone(), specs));
    let bus: Arc<Bus<std::os::unix::net::UnixStream>> = Arc::default();
    jobs.start_at_load();

    spawn_qos();
    // The compositor and shell start after wold: re-check every few seconds (idempotent).
    std::thread::spawn(|| {
        loop {
            qos::protect_session(Path::new("/proc"));
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
    {
        let (jobs, bus) = (jobs.clone(), bus.clone());
        let grants = Arc::new(api::portal::ConsentFile(wol_core::consent::FILE.into()));
        let notes = Arc::new(api::portal::Notes::default());
        let listener =
            server::bind(Path::new(shared::jobs::SOCKET), 0o666).expect("bind control socket");
        eprintln!("wold: control listening on {}", shared::jobs::SOCKET);
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let peer = shared::sys::peer(&conn);
                let (jobs, bus, grants, notes) =
                    (jobs.clone(), bus.clone(), grants.clone(), notes.clone());
                std::thread::spawn(move || {
                    server::serve(conn, peer, &*jobs, &bus, &*grants, &notes)
                });
            }
        });
    }

    loop {
        match os::wait_signal() {
            libc::SIGCHLD => {
                ctx.reaper.reap_all(|p| jobs.is_job(p));
            }
            _ => {
                ctx.stopping
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                os::stop_all(&jobs.running());
                std::process::exit(0);
            }
        }
    }
}

fn spawn_qos() {
    let listener = match server::bind(Path::new(shared::qos::SOCKET), 0o666) {
        Ok(l) => l,
        Err(e) => return eprintln!("wold: qos disabled: {e}"),
    };
    let allowed = shared::sys::canonical(qos::REALTIME_EXE);
    let tracker = Arc::new(Mutex::new(shared::qos::ForegroundTracker::default()));
    let n = qos::hint_defaults(Path::new(qos::HINTS));
    eprintln!(
        "wold: qos listening on {} ({n} kernel hints)",
        shared::qos::SOCKET
    );
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let mut exe = [0u8; 1024];
            let ok = shared::sys::peer(&conn)
                .and_then(|p| shared::sys::executable(p.pid, &mut exe))
                .is_some_and(|n| qos::peer_allowed(&exe[..n], &allowed));
            if !ok {
                eprintln!("wold: qos refused peer");
                continue;
            }
            let tracker = tracker.clone();
            std::thread::spawn(move || qos::serve(conn, &tracker, &qos::Sched));
        }
    });
}
