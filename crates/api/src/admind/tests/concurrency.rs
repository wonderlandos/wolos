// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-admind requests from many threads at once (api-security 1+2): no refusal delay, password
//! check or long command holds another request, revokes are per session, and what must run one
//! at a time still does.
use super::*;
use shared::admin::encode;

/// A thread-safe fake (connections run on their own threads) whose refusal delay, password
/// check and pacman runs all wait at one gate until the test opens it: "slow" without any
/// sleeping, and a request that gets through while they wait was not held by them.
#[derive(Default)]
struct Slow {
    delays: std::sync::atomic::AtomicU32,
    /// (slow calls started, gate open, clock reads).
    gate: Mutex<(u32, bool, u32)>,
    moved: std::sync::Condvar,
    /// Seconds added to the fake clock (100).
    later: std::sync::atomic::AtomicU64,
}

/// How long a test waits for something that should happen at once.
const PROMPT: std::time::Duration = std::time::Duration::from_secs(5);

impl Slow {
    fn wait(&self) {
        let mut g = lock(&self.gate);
        g.0 += 1;
        self.moved.notify_all();
        while !g.1 {
            g = self.moved.wait(g).unwrap_or_else(|p| p.into_inner());
        }
    }
    /// Until a slow call is waiting at the gate.
    fn started(&self) {
        let g = lock(&self.gate);
        let (_g, t) = self
            .moved
            .wait_timeout_while(g, PROMPT, |g| g.0 == 0)
            .unwrap_or_else(|p| p.into_inner());
        assert!(!t.timed_out(), "nothing slow started");
    }
    fn open(&self) {
        lock(&self.gate).1 = true;
        self.moved.notify_all();
    }
    /// Slow calls started so far.
    fn slow_calls(&self) -> u32 {
        lock(&self.gate).0
    }
    /// True if no more than `n` slow calls start within `d` (a call that should be held
    /// back is: under a missing lock it would reach the gate at once).
    fn held(&self, n: u32, d: std::time::Duration) -> bool {
        let g = lock(&self.gate);
        let (g, t) = self
            .moved
            .wait_timeout_while(g, d, |g| g.0 <= n)
            .unwrap_or_else(|p| p.into_inner());
        t.timed_out() && g.0 == n
    }
    fn clock_reads(&self) -> u32 {
        lock(&self.gate).2
    }
    /// Until the clock has been read more than `n` times.
    fn read_clock_after(&self, n: u32) {
        let g = lock(&self.gate);
        let (_g, t) = self
            .moved
            .wait_timeout_while(g, PROMPT, |g| g.2 <= n)
            .unwrap_or_else(|p| p.into_inner());
        assert!(!t.timed_out(), "the clock was not read");
    }
}

/// `handle` on a thread of its own, its reply awaited at most [`PROMPT`] (a request held by
/// another fails the test instead of hanging it).
fn promptly(
    data: &[u8],
    caller: Option<Caller>,
    shared: &Arc<Shared>,
    os: &'static Slow,
) -> Option<AdminReply> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (data, shared) = (data.to_vec(), shared.clone());
    std::thread::spawn(move || tx.send(handle(&data, caller, &shared, os)));
    rx.recv_timeout(PROMPT).ok()
}

impl AdminOs for Slow {
    fn passwd(&self) -> String {
        super::Fake::default().passwd()
    }
    fn group(&self) -> String {
        super::Fake::default().group()
    }
    fn authenticate(&self, user: &str, pw: &[u8]) -> bool {
        self.wait();
        user == "ann" && pw == b"good"
    }
    fn run(&self, argv: &[String], _: bool) -> bool {
        // pacman (Update Now runs it in place here, see `read`), and the update check PkgRefresh
        // runs (it refreshes a copy of the database).
        if argv[0] == "pacman"
            || argv[0] == wol_core::update::PACMAN
            || argv[0] == wol_core::update::check::SCRIPT
        {
            self.wait();
        }
        true
    }
    fn run_input(&self, _: &[String], _: &str) -> bool {
        true
    }
    fn run_progress(&self, argv: &[String], _: wol_core::progress::Kind, _: &str) -> bool {
        self.run(argv, true)
    }
    fn write(&self, _: &str, _: u32, _: &str) -> bool {
        true
    }
    /// One copy of WolOS (ADR 0023): Update Now (`update_now`) is one pacman run, no slot.
    fn read(&self, path: &str) -> Option<String> {
        (path == wol_core::update::SINGLE_COPY).then(String::new)
    }
    fn refuse_delay(&self) {
        self.delays
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.wait();
    }
    fn now(&self) -> u64 {
        lock(&self.gate).2 += 1;
        self.moved.notify_all();
        100 + self.later.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn log(&self, _: &str) {}
    fn sessions(&self) -> Vec<u32> {
        Vec::new()
    }
    fn capture(&self, _: &[String]) -> Option<String> {
        None
    }
    fn wall_now(&self) -> u64 {
        0
    }
    fn stage_dir(&self) -> Option<PathBuf> {
        None
    }
    fn copy_in(&self, _: &str, _: &Path, _: Option<u32>) -> Result<(), String> {
        Err("no".into())
    }
    fn trusted_keys(&self) -> Vec<[u8; 32]> {
        Vec::new()
    }
    fn remove_dir(&self, _: &Path) {}
    fn package_facts(&self, _: &str) -> Result<crate::packages::PkgFacts, String> {
        Err("no".into())
    }
}

/// api-security 1+2: while one account's refusal delay, wrong-password check or long install
/// is still under way, valid requests from other threads are answered; the refusal still
/// waited its delay (one per refusal).
#[test]
fn a_refusal_a_password_check_or_an_install_does_not_hold_other_requests() {
    use std::sync::atomic::Ordering::SeqCst;
    let upgrade = |pw: &[u8]| encode(&req(AdminAction::PkgUpgrade, &[]), pw);
    let quick = encode(&req(AdminAction::Hostname, &["wolbox"]), b"");
    for (case, slow_req, slow_caller, slow_reply, delays) in [
        ("refusal", b"garbage".to_vec(), BOB, AdminReply::Invalid, 1),
        ("password", upgrade(b"bad"), ANN, AdminReply::Denied, 1),
        ("install", upgrade(b""), ROOT, AdminReply::Ok, 0),
    ] {
        let os: &'static Slow = Box::leak(Box::default());
        let shared = Arc::new(Shared::default());
        let slow = {
            let shared = shared.clone();
            std::thread::spawn(move || handle(&slow_req, slow_caller, &shared, os))
        };
        os.started();
        for _ in 0..3 {
            assert_eq!(
                promptly(&quick, ROOT, &shared, os),
                Some(AdminReply::Ok),
                "{case}: held by the slow request"
            );
        }
        os.open();
        assert_eq!(slow.join().unwrap(), slow_reply, "{case}");
        assert_eq!(
            os.delays.load(SeqCst),
            delays,
            "{case}: one delay per refusal"
        );
    }
}

/// A revoke that arrives while the password check is still running wins: the accepted
/// password runs its request but leaves no cached authorization behind.
#[test]
fn a_revoke_during_a_password_check_leaves_no_grant() {
    let os: &'static Slow = Box::leak(Box::default());
    let shared = Arc::new(Shared::default());
    let up = |pw: &[u8]| encode(&req(AdminAction::PkgUpgrade, &[]), pw);
    let checking = {
        let shared = shared.clone();
        std::thread::spawn(move || handle(&up(b"good"), ANN, &shared, os))
    };
    os.started();
    let revoke = encode(&req(AdminAction::Revoke, &[]), b"");
    assert_eq!(promptly(&revoke, ANN, &shared, os), Some(AdminReply::Ok));
    os.open();
    assert_eq!(checking.join().unwrap(), AdminReply::Ok);
    assert_eq!(handle(&up(b""), ANN, &shared, os), AdminReply::NeedsAuth);
}

/// Review of api-security 1: a revoke is per session. Another account (or another session
/// of the same one) revoking while ann's password is checked leaves ann's grant in place.
#[test]
fn a_revoke_elsewhere_does_not_cancel_a_grant() {
    let up = |pw: &[u8]| encode(&req(AdminAction::PkgUpgrade, &[]), pw);
    let revoke = encode(&req(AdminAction::Revoke, &[]), b"");
    let ann_elsewhere = Some(Caller {
        uid: 1001,
        session: 9,
    });
    for other in [BOB, ann_elsewhere] {
        let os: &'static Slow = Box::leak(Box::default());
        let shared = Arc::new(Shared::default());
        let checking = {
            let shared = shared.clone();
            std::thread::spawn(move || handle(&up(b"good"), ANN, &shared, os))
        };
        os.started();
        assert_eq!(promptly(&revoke, other, &shared, os), Some(AdminReply::Ok));
        os.open();
        assert_eq!(checking.join().unwrap(), AdminReply::Ok);
        assert_eq!(
            handle(&up(b""), ANN, &shared, os),
            AdminReply::Ok,
            "{other:?}: ann's grant stays"
        );
    }
}

/// Review of api-security 2: a command queued behind an install on the cached
/// authorization runs only if that still holds when its turn comes: a revoke of the session
/// during the wait, or the cache running out, stops it (the sheet asks again).
#[test]
fn a_queued_command_on_the_cache_rechecks_it_when_its_turn_comes() {
    use std::sync::atomic::Ordering::SeqCst;
    let up = |pw: &[u8]| encode(&req(AdminAction::PkgUpgrade, &[]), pw);
    let revoke = encode(&req(AdminAction::Revoke, &[]), b"");
    for case in ["revoke", "expiry", "still valid"] {
        let os: &'static Slow = Box::leak(Box::default());
        let shared = Arc::new(Shared::default());
        shared.cached().cache.grant(1001, 7, 100);
        let install = {
            let shared = shared.clone();
            std::thread::spawn(move || handle(&up(b""), ROOT, &shared, os))
        };
        os.started(); // root's install holds the system lane
        let reads = os.clock_reads();
        let queued = {
            let shared = shared.clone();
            std::thread::spawn(move || handle(&up(b""), ANN, &shared, os))
        };
        os.read_clock_after(reads); // ann was judged on the cache and waits for the lane
        match case {
            "revoke" => {
                assert_eq!(promptly(&revoke, ANN, &shared, os), Some(AdminReply::Ok));
            }
            "expiry" => os.later.store(wol_core::admin::CACHE_SECS, SeqCst),
            _ => {}
        }
        os.open();
        assert_eq!(install.join().unwrap(), AdminReply::Ok);
        let (want, ran) = if case == "still valid" {
            (AdminReply::Ok, 2)
        } else {
            (AdminReply::NeedsAuth, 1)
        };
        assert_eq!(queued.join().unwrap(), want, "{case}");
        assert_eq!(os.slow_calls(), ran, "{case}: pacman runs");
    }
}

/// Two system-lane commands run one after the other (one pacman lock): the second does
/// not start while the first runs.
#[test]
fn system_lane_commands_run_one_at_a_time() {
    let os: &'static Slow = Box::leak(Box::default());
    let shared = Arc::new(Shared::default());
    let first = {
        let shared = shared.clone();
        let r = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
        std::thread::spawn(move || handle(&r, ROOT, &shared, os))
    };
    os.started();
    let second = {
        let shared = shared.clone();
        let r = encode(&req(AdminAction::PkgRefresh, &[]), b"");
        std::thread::spawn(move || handle(&r, ROOT, &shared, os))
    };
    assert!(
        os.held(1, std::time::Duration::from_millis(300)),
        "the second pacman started beside the first"
    );
    os.open();
    assert_eq!(first.join().unwrap(), AdminReply::Ok);
    assert_eq!(second.join().unwrap(), AdminReply::Ok);
    assert_eq!(os.slow_calls(), 2);
}

/// Two password checks for one account (two sessions of ann) run one after the other, so
/// more connections never mean faster guessing.
#[test]
fn one_accounts_password_checks_run_one_at_a_time() {
    let os: &'static Slow = Box::leak(Box::default());
    let shared = Arc::new(Shared::default());
    let ask = |caller: Option<Caller>, pw: &'static [u8]| {
        let shared = shared.clone();
        let r = encode(&req(AdminAction::Hostname, &["wolbox"]), pw);
        std::thread::spawn(move || handle(&r, caller, &shared, os))
    };
    let first = ask(ANN, b"bad");
    os.started();
    let second = ask(
        Some(Caller {
            uid: 1001,
            session: 9,
        }),
        b"good",
    );
    assert!(
        os.held(1, std::time::Duration::from_millis(300)),
        "a second password check of ann ran beside the first"
    );
    os.open();
    assert_eq!(first.join().unwrap(), AdminReply::Denied);
    assert_eq!(second.join().unwrap(), AdminReply::Ok);
}
