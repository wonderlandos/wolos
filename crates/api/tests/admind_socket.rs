// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-admind's accept loop on real sockets (QA-100 SHADOW-20, SHADOW-41): what a client sees
//! when the service is out of descriptors or the account is over its connection limit.
use api::admind::{self, AdminOs};
use shared::admin::{AdminAction, AdminReply, AdminRequest, encode};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// An OS that approves nothing and logs to memory.
#[derive(Default)]
struct Quiet {
    log: Mutex<Vec<String>>,
}

impl AdminOs for Quiet {
    fn passwd(&self) -> String {
        "root:*:0:0::/root:/bin/sh\n".into()
    }
    fn group(&self) -> String {
        String::new()
    }
    fn authenticate(&self, _: &str, _: &[u8]) -> bool {
        false
    }
    fn run(&self, _: &[String], _: bool) -> bool {
        false
    }
    fn run_input(&self, _: &[String], _: &str) -> bool {
        false
    }
    fn run_progress(&self, _: &[String], _: wol_core::progress::Kind, _: &str) -> bool {
        false
    }
    fn write(&self, _: &str, _: u32, _: &str) -> bool {
        false
    }
    fn read(&self, _: &str) -> Option<String> {
        None
    }
    fn refuse_delay(&self) {}
    fn now(&self) -> u64 {
        0
    }
    fn log(&self, text: &str) {
        self.log.lock().unwrap().push(text.to_owned());
    }
    fn sessions(&self) -> Vec<u32> {
        Vec::new()
    }
    fn capture(&self, _: &[String]) -> Option<String> {
        None
    }
    fn wall_now(&self) -> u64 {
        0
    }
    fn stage_dir(&self) -> Option<std::path::PathBuf> {
        None
    }
    fn copy_in(&self, _: &str, _: &std::path::Path, _: Option<u32>) -> Result<(), String> {
        Err(String::new())
    }
    fn trusted_keys(&self) -> Vec<[u8; 32]> {
        Vec::new()
    }
    fn remove_dir(&self, _: &std::path::Path) {}
    fn package_facts(&self, _: &str) -> Result<api::packages::PkgFacts, String> {
        Err(String::new())
    }
}

fn leak() -> &'static Quiet {
    Box::leak(Box::default())
}

fn sock(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wol-admind-socket-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.join("admin.sock")
}

/// QA-100 SHADOW-20: out of descriptors, `accept` fails again and again. The loop waits between
/// tries (it does not spin) and says so once in the log, not once per try.
#[test]
fn accept_emfile_logged_not_looped() {
    let os = leak();
    let tries = std::sync::atomic::AtomicUsize::new(0);
    let start = Instant::now();
    admind::serve_with(
        || {
            tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(std::io::Error::from_raw_os_error(libc::EMFILE))
        },
        os,
        || tries.load(std::sync::atomic::Ordering::SeqCst) >= 5,
    );
    let took = start.elapsed();
    assert!(
        took >= admind::ACCEPT_BACKOFF * 4,
        "5 failed accepts took {took:?}: the loop spun"
    );
    let log = os.log.lock().unwrap();
    let said: Vec<_> = log.iter().filter(|l| l.contains("accept failed")).collect();
    assert_eq!(said.len(), 1, "logged once, not once per try: {log:?}");
    assert!(
        said[0].contains("Too many open files"),
        "the error itself is in the line: {}",
        said[0]
    );
}

/// The hello answered, then `req` sent and the write side closed; the reply line.
fn ask(path: &std::path::Path, req: &AdminRequest) -> std::io::Result<String> {
    let mut s = shared::jobs::connect(path)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    s.write_all(&encode(req, b""))?;
    s.shutdown(std::net::Shutdown::Write)?;
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line)?;
    Ok(line)
}

/// QA-100 SHADOW-41: the account's fifth connection (the limit is [`admind::CONN_PER_UID`]) is
/// greeted and told the service is busy, not dropped (the client used to say "administrator
/// service unavailable" while the service ran). The connections within the limit are served.
#[test]
fn over_quota_replies_busy() {
    let os = leak();
    let path = sock("quota");
    let listener = UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || admind::serve(&listener, os));
    // CONN_PER_UID connections that answer the hello and then say nothing: they hold their places
    // until the request deadline.
    let held: Vec<UnixStream> = (0..admind::CONN_PER_UID)
        .map(|_| shared::jobs::connect(&path).expect("within the limit"))
        .collect();
    // `connect` returns once the hello is answered, and the daemon admits a connection before it
    // greets: all four hold a place by now, and the next one is judged after them.
    let req = AdminRequest {
        action: AdminAction::Revoke,
        args: vec![],
    };
    let line = ask(&path, &req).expect("a reply, not a closed socket");
    let msg = shared::admin::failure_message(&line);
    assert_eq!(msg.as_deref(), Some(admind::BUSY), "reply line: {line:?}");
    assert_eq!(AdminReply::parse(&line), AdminReply::Failed);
    // The four are served as ever: each gets its answer once it has said its request.
    for mut s in held {
        s.write_all(&encode(&req, b"")).unwrap();
        s.shutdown(std::net::Shutdown::Write).unwrap();
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "ok", "within the limit");
    }
    // Their places come back as the daemon finishes with them: the same request is served again.
    let mut again = String::new();
    for _ in 0..1_000_000 {
        again = ask(&path, &req).unwrap();
        if again.trim() == "ok" {
            break;
        }
        std::thread::yield_now();
    }
    assert_eq!(again.trim(), "ok", "places given back");
    let log = os.log.lock().unwrap();
    assert_eq!(
        log.iter()
            .filter(|l| l.contains("over its connection limit"))
            .count(),
        1,
        "{log:?}"
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
