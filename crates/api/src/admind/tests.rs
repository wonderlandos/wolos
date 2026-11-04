// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-admind request handling against an in-memory OS.
use super::*;
use shared::admin::{AdminRequest, encode};
use std::cell::{Cell, RefCell};

#[derive(Default)]
struct Fake {
    log: RefCell<Vec<String>>,
    ran: RefCell<Vec<Vec<String>>>,
    stdin: RefCell<Vec<String>>,
    wrote: RefCell<Vec<(String, String)>>,
    delays: Cell<u32>,
    /// A refusal delay that says it started and then waits for the release (QA-100 H1).
    hold_refusal: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
    auths: Cell<u32>,
    fail_run: bool,
    /// A run whose command line contains this fails (one step of a sequence).
    fail_on: Option<&'static str>,
    /// What the failed step said on stderr.
    stderr: &'static str,
    /// Another update holds the update lock.
    locked: bool,
    /// The signed newer release the daily check found: (version, snapshot date).
    release: Option<(&'static str, &'static str)>,
    captured: RefCell<Vec<Vec<String>>>,
    /// Probe answers by command line (`sysctl -n hw.ncpu` -> "4\n"); others fail.
    probes: Vec<(&'static str, &'static str)>,
    /// Accounts with a desktop session.
    sessions: Vec<u32>,

    /// Trusted package keys.
    keys: Vec<[u8; 32]>,
    /// (`pacman -Q` command line suffix, stdout); unmatched queries fail.
    pkg: Vec<(&'static str, String)>,
    /// (path, text) for `read`.
    files: Vec<(String, String)>,
    stage: RefCell<Option<std::path::PathBuf>>,
    /// Progress files of tracked runs.
    progress: RefCell<Vec<String>>,
    /// SHA-256 of every staged package copy (None: [`FAKE_SHA`]).
    staged_sha: Option<String>,
}

/// What the fake reads from a staged package copy unless `staged_sha` says otherwise.
const FAKE_SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

impl AdminOs for Fake {
    fn passwd(&self) -> String {
        "root:*:0:0::/root:/bin/sh\nann:*:1001:1001::/home/ann:/bin/sh\nbob:*:1002:1002::/home/bob:/bin/sh\n".into()
    }
    fn group(&self) -> String {
        "wheel:*:0:root,ann\n".into()
    }
    fn authenticate(&self, user: &str, pw: &[u8]) -> bool {
        self.auths.set(self.auths.get() + 1);
        user == "ann" && pw == b"good"
    }
    fn run(&self, argv: &[String], _: bool) -> bool {
        self.ran.borrow_mut().push(argv.to_vec());
        !self.fail_run && self.fail_on.is_none_or(|f| !argv.join(" ").contains(f))
    }
    fn run_progress(&self, argv: &[String], _: wol_core::progress::Kind, path: &str) -> bool {
        self.progress.borrow_mut().push(path.into());
        self.run(argv, true)
    }
    fn last_error(&self) -> String {
        self.stderr.into()
    }
    fn update_locked(&self) -> bool {
        self.locked
    }
    fn trusted_release(&self) -> Option<(String, String)> {
        self.release.map(|(v, d)| (v.into(), d.into()))
    }
    fn capture(&self, argv: &[String]) -> Option<String> {
        self.captured.borrow_mut().push(argv.to_vec());
        let line = argv.join(" ");
        if argv[0] == "pacman" {
            return self
                .pkg
                .iter()
                .find(|(k, _)| line.ends_with(k))
                .map(|(_, v)| v.clone());
        }
        self.probes
            .iter()
            .find(|p| p.0 == line)
            .map(|p| p.1.to_string())
    }
    fn stage_dir(&self) -> Option<std::path::PathBuf> {
        let d = std::env::temp_dir().join(format!(
            "wol-admind-stage-{}-{:p}",
            std::process::id(),
            self
        ));
        std::fs::create_dir_all(&d).ok()?;
        *self.stage.borrow_mut() = Some(d.clone());
        Some(d)
    }
    fn copy_in(&self, src: &str, dst: &std::path::Path, _: Option<u32>) -> Result<(), String> {
        std::fs::copy(src, dst)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn trusted_keys(&self) -> Vec<[u8; 32]> {
        self.keys.clone()
    }
    fn remove_dir(&self, d: &std::path::Path) {
        let _ = std::fs::remove_dir_all(d);
    }
    fn package_facts(&self, _: &str) -> Result<crate::packages::PkgFacts, String> {
        match self.staged_sha.as_deref() {
            Some("") => Err("it has no package information (.PKGINFO) at its start".into()),
            sha => Ok(crate::packages::PkgFacts {
                name: "x".into(),
                version: "1-1".into(),
                sha256: sha.unwrap_or(FAKE_SHA).into(),
            }),
        }
    }
    fn wall_now(&self) -> u64 {
        1_761_600_000
    }
    fn run_input(&self, argv: &[String], input: &str) -> bool {
        self.ran.borrow_mut().push(argv.to_vec());
        self.stdin.borrow_mut().push(input.into());
        !self.fail_run && self.fail_on.is_none_or(|f| !argv.join(" ").contains(f))
    }
    fn write(&self, path: &str, _: u32, text: &str) -> bool {
        self.wrote.borrow_mut().push((path.into(), text.into()));
        true
    }
    fn read(&self, path: &str) -> Option<String> {
        self.files.iter().find(|f| f.0 == path).map(|f| f.1.clone())
    }
    fn refuse_delay(&self) {
        self.delays.set(self.delays.get() + 1);
        if let Some((started, release)) = &self.hold_refusal {
            let _ = started.send(());
            let _ = release.recv_timeout(std::time::Duration::from_secs(20));
        }
    }
    fn now(&self) -> u64 {
        100
    }
    fn log(&self, t: &str) {
        self.log.borrow_mut().push(t.into());
    }
    fn sessions(&self) -> Vec<u32> {
        self.sessions.clone()
    }
}

/// A machine that boots root slot wolos-a; wolos-b holds the previous restore point.
const SLOT_CMDLINE: &str = "root=LABEL=wolos rootflags=subvol=@wolos-a rw quiet";
const SLOT_BOOTCTL: &str = concat!(
    r#"[{"id":"wolos-a.conf","isSelected":true,"isDefault":true,"version":"0.12-2026/11/02"},"#,
    r#"{"id":"wolos-b.conf","isSelected":false,"isDefault":false}]"#
);

fn slot_os() -> Fake {
    Fake {
        files: vec![("/proc/cmdline".into(), SLOT_CMDLINE.into())],
        probes: vec![("bootctl list --json=short", SLOT_BOOTCTL)],
        ..Fake::default()
    }
}

fn req(a: AdminAction, args: &[&str]) -> AdminRequest {
    AdminRequest {
        action: a,
        args: args.iter().map(|s| s.to_string()).collect(),
    }
}

const ANN: Option<Caller> = Some(Caller {
    uid: 1001,
    session: 7,
});
const BOB: Option<Caller> = Some(Caller {
    uid: 1002,
    session: 8,
});
const ROOT: Option<Caller> = Some(Caller { uid: 0, session: 1 });

#[test]
fn decision_table() {
    let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
    let up_good = encode(&req(AdminAction::PkgUpgrade, &[]), b"good");
    let up_bad = encode(&req(AdminAction::PkgUpgrade, &[]), b"bad");
    let bad = encode(&req(AdminAction::Idle, &["x"]), b"");
    let cases: [(&[u8], Option<Caller>, AdminReply, u32); 8] = [
        (&up, ROOT, AdminReply::Ok, 0),
        (&up, ANN, AdminReply::NeedsAuth, 0),
        (&up_bad, ANN, AdminReply::Denied, 1),
        (&up_good, ANN, AdminReply::Ok, 0),
        (&up_good, BOB, AdminReply::NotAdmin, 1),
        (&up, None, AdminReply::NotAdmin, 1),
        (&bad, ROOT, AdminReply::Invalid, 1),
        (b"garbage", ROOT, AdminReply::Invalid, 1),
    ];
    for (data, caller, want, delays) in cases {
        let os = slot_os();
        let got = handle(data, caller, &Shared::default(), &os);
        assert_eq!(
            (got, os.delays.get()),
            (want, delays),
            "{caller:?} {:?}",
            String::from_utf8_lossy(data)
        );
    }
}

/// Finish Secure Boot setup from System Settings: an administrator's password, then the one
/// fixed command (`wol boot enroll`, which prints `enrolled` and changes nothing when the key
/// is already in db, so asking twice is harmless).
#[test]
fn boot_enroll_needs_the_admin_password_and_runs_the_verb() {
    let os = Fake::default();
    let cache = Shared::default();
    let none = encode(&req(AdminAction::BootEnroll, &[]), b"");
    let good = encode(&req(AdminAction::BootEnroll, &[]), b"good");
    assert_eq!(handle(&none, ANN, &cache, &os), AdminReply::NeedsAuth);
    assert_eq!(handle(&good, BOB, &cache, &os), AdminReply::NotAdmin);
    assert_eq!(handle(&good, ANN, &cache, &os), AdminReply::Ok);
    assert_eq!(handle(&good, ANN, &cache, &os), AdminReply::Ok);
    let ran: Vec<String> = os.ran.borrow().iter().map(|v| v.join(" ")).collect();
    assert_eq!(ran, ["wol boot enroll", "wol boot enroll"]);
    let with_arg = encode(&req(AdminAction::BootEnroll, &["--force"]), b"good");
    assert_eq!(
        handle(&with_arg, ROOT, &cache, &Fake::default()),
        AdminReply::Invalid
    );
}

/// Audit C7-1 (ADR 0034): a locally built package, Undo Update and the update channel run as
/// root only for an administrator, with a password typed for each (never the session cache).
/// A standard account is refused whatever its own shell answered; a path outside wold's
/// staging layout is refused before anything runs.
#[test]
fn root_package_changes_need_an_administrator_and_a_fresh_password() {
    let staged = format!(
        "{}/0123456789abcdef0123456789abcdef/x-1-1-x86_64.pkg.tar.zst",
        shared::package::STAGING_DIR
    );
    let os = Fake::default();
    let cache = Shared::default();
    for (action, args) in [
        (AdminAction::PkgInstallFile, vec![staged.as_str(), FAKE_SHA]),
        (AdminAction::SystemRollback, vec![]),
        (AdminAction::SystemChannel, vec!["beta"]),
    ] {
        let good = encode(&req(action, &args), b"good");
        let none = encode(&req(action, &args), b"");
        assert_eq!(handle(&good, BOB, &cache, &os), AdminReply::NotAdmin);
        assert_eq!(handle(&good, ANN, &cache, &os), AdminReply::Ok);
        assert_eq!(
            handle(&none, ANN, &cache, &os),
            AdminReply::NeedsAuth,
            "{action:?}: a warm cache is not enough"
        );
    }
    let ran: Vec<String> = os.ran.borrow().iter().map(|v| v.join(" ")).collect();
    assert_eq!(
        ran,
        [
            format!("wol pkg -- -U --noconfirm -- {staged}"),
            "wol update rollback".into()
        ]
    );
    assert_eq!(
        *os.wrote.borrow(),
        [(
            wol_core::admin::CHANNEL_FILE.to_owned(),
            "beta\n".to_owned()
        )]
    );
    for path in ["/home/bob/evil-1-1-any.pkg.tar.zst", "/tmp/x.pkg.tar.zst"] {
        let r = encode(
            &req(AdminAction::PkgInstallFile, &[path, FAKE_SHA]),
            b"good",
        );
        assert_eq!(handle(&r, ANN, &cache, &os), AdminReply::Invalid);
    }
    assert_eq!(os.ran.borrow().len(), 2, "nothing else ran");
    assert!(
        os.log
            .borrow()
            .iter()
            .any(|l| l == &format!("ann(1001): pkg-install-file: x 1-1 sha256 {FAKE_SHA}")),
        "{:?}",
        os.log.borrow()
    );
}

/// ADR 0034: wol-admind installs only the copy the sheet described. A staged copy whose
/// SHA-256 is not the one shown, or that cannot be read, is not installed, and the log says
/// why.
#[test]
fn a_staged_package_that_is_not_what_the_sheet_showed_is_not_installed() {
    let staged = format!(
        "{}/0123456789abcdef0123456789abcdef/x-1-1-x86_64.pkg.tar.zst",
        shared::package::STAGING_DIR
    );
    let cache = Shared::default();
    for (sha, why) in [
        ("f".repeat(64), "the sheet showed"),
        (String::new(), ".PKGINFO"),
    ] {
        let os = Fake {
            staged_sha: Some(sha),
            ..Default::default()
        };
        let r = encode(
            &req(AdminAction::PkgInstallFile, &[&staged, FAKE_SHA]),
            b"good",
        );
        assert_eq!(handle(&r, ANN, &cache, &os), AdminReply::Failed);
        assert!(os.ran.borrow().is_empty(), "nothing ran");
        assert!(
            os.log.borrow().iter().any(|l| l.contains(why)),
            "{:?}",
            os.log.borrow()
        );
    }
}

/// QA-100 H1: one account's refusal delay (2 s each, looped by any local program) does
/// not hold the cache lock: another account's valid request is answered meanwhile.
#[test]
fn blocking_refuse_delay_does_not_stall_valid_request() {
    use std::sync::mpsc::channel;
    let cache = Shared::default();
    let (started_tx, started_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (done_tx, done_rx) = channel();
    std::thread::scope(|sc| {
        let cache = &cache;
        sc.spawn(move || {
            let os = Fake {
                hold_refusal: Some((started_tx, release_rx)),
                ..Fake::default()
            };
            let r = encode(&req(AdminAction::PkgUpgrade, &[]), b"good");
            let reply = handle_said(&r, BOB, cache, &os, &mut None);
            let _ = done_tx.send(reply);
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("bob's refusal started");
        let os = slot_os();
        let good = encode(&req(AdminAction::PkgUpgrade, &[]), b"good");
        // Under the old single lock this call waited for bob's delay (forever here).
        let (tx, rx) = channel();
        sc.spawn(move || {
            let _ = tx.send(handle_said(&good, ANN, cache, &os, &mut None));
        });
        let ann = rx.recv_timeout(std::time::Duration::from_secs(5));
        let _ = release_tx.send(());
        assert_eq!(ann, Ok(AdminReply::Ok), "ann was stalled by bob's refusal");
        assert_eq!(
            done_rx.recv_timeout(std::time::Duration::from_secs(10)),
            Ok(AdminReply::NotAdmin)
        );
    });
    assert!(cache.cached().cache.valid(1001, 7, 100), "ann's grant kept");
}

#[test]
fn authorization_is_cached_per_session_and_revocable() {
    let os = slot_os();
    let cache = Shared::default();
    let good = encode(&req(AdminAction::PkgUpgrade, &[]), b"good");
    let none = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
    assert_eq!(handle(&good, ANN, &cache, &os), AdminReply::Ok);
    assert_eq!(handle(&none, ANN, &cache, &os), AdminReply::Ok);
    assert_eq!(os.auths.get(), 1);
    let other_session = Some(Caller {
        uid: 1001,
        session: 9,
    });
    assert_eq!(
        handle(&none, other_session, &cache, &os),
        AdminReply::NeedsAuth
    );
    assert_eq!(
        handle(
            &encode(&req(AdminAction::Revoke, &[]), b""),
            ANN,
            &cache,
            &os
        ),
        AdminReply::Ok
    );
    assert_eq!(handle(&none, ANN, &cache, &os), AdminReply::NeedsAuth);
}

/// Settings > Modules Install sends two `PkgInstall` (the backend package, then the meta
/// package): the person types the password for the first only; the second rides the session's
/// authorization, so ONE password sheet rises (and a wrong password stops before either runs).
#[test]
fn a_module_install_of_two_packages_asks_one_password() {
    let os = Fake::default();
    let cache = Shared::default();
    let send = |name: &str, pw: &[u8], cache: &Shared| {
        handle(
            &encode(&req(AdminAction::PkgInstall, &[name]), pw),
            ANN,
            cache,
            &os,
        )
    };
    assert_eq!(send("ollama-rocm", b"", &cache), AdminReply::NeedsAuth);
    assert_eq!(send("ollama-rocm", b"bad", &cache), AdminReply::Denied);
    assert!(
        os.ran.borrow().is_empty(),
        "nothing ran without the password"
    );
    assert_eq!(send("ollama-rocm", b"good", &cache), AdminReply::Ok);
    assert_eq!(
        send("wolos-module-ai-local", b"", &cache),
        AdminReply::Ok,
        "the second package needs no second sheet"
    );
    assert_eq!(
        os.auths.get(),
        2,
        "one failed and one good password, no more"
    );
    assert_eq!(os.ran.borrow().len(), 2);
}

#[test]
fn runs_planned_steps_and_logs_without_secrets() {
    let os = Fake::default();
    let r = req(AdminAction::WifiJoin, &["wlan0", "Home", "hunter22"]);
    assert_eq!(
        handle(&encode(&r, b""), ROOT, &Shared::default(), &os),
        AdminReply::Ok
    );
    assert!(os.wrote.borrow().is_empty());
    // NET2-1: a profile, then `connection up ... passwd-file` (the password on stdin).
    assert_eq!(
        os.ran.borrow()[0].join(" "),
        "nmcli connection add type wifi con-name Home ifname * ssid Home wifi-sec.key-mgmt wpa-psk"
    );
    assert_eq!(
        os.ran.borrow()[1].join(" "),
        "nmcli connection up id Home passwd-file /dev/stdin"
    );
    assert!(os.log.borrow().iter().all(|l| !l.contains("hunter22")));
    let failing = Fake {
        fail_run: true,
        ..slot_os()
    };
    let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
    assert_eq!(
        handle(&up, ROOT, &Shared::default(), &failing),
        AdminReply::Failed
    );
}

#[test]
fn user_add_sets_password_on_stdin() {
    let os = Fake::default();
    let r = req(
        AdminAction::UserAdd,
        &["dinah", "Dinah", "no", "n3w-passw0rd"],
    );
    assert_eq!(
        handle(&encode(&r, b""), ROOT, &Shared::default(), &os),
        AdminReply::Ok
    );
    let ran = os.ran.borrow();
    assert_eq!(ran[0][..2], ["useradd", "-m"]);
    assert_eq!(ran[1], ["chpasswd"]);
    assert_eq!(os.stdin.borrow()[0], "dinah:n3w-passw0rd\n");
    assert!(ran.iter().flatten().all(|a| !a.contains("n3w")));
    assert!(os.log.borrow().iter().all(|l| !l.contains("n3w")));
    let nopw = encode(&req(AdminAction::UserAdd, &["dinah", "Dinah", "no"]), b"");
    assert_eq!(
        handle(&nopw, ROOT, &Shared::default(), &os),
        AdminReply::Invalid
    );
}

/// Changing a password: anyone their own, always with the current one; someone else's only
/// an administrator. The account and the disk slot get the new one on stdin.
#[test]
fn a_password_change_needs_the_current_password_every_time() {
    let os = Fake::default();
    let cache = Shared::default();
    let mine = req(AdminAction::UserPassword, &["ann", "n3w-passw0rd"]);
    assert_eq!(
        handle(&encode(&mine, b"good"), ANN, &cache, &os),
        AdminReply::Ok
    );
    assert_eq!(os.ran.borrow()[0], ["chpasswd"]);
    assert_eq!(os.ran.borrow()[1], ["wol", "encrypt", "password", "ann"]);
    assert_eq!(*os.stdin.borrow(), ["ann:n3w-passw0rd\n", "n3w-passw0rd\n"]);
    assert!(os.log.borrow().iter().all(|l| !l.contains("n3w")));
    assert_eq!(
        handle(&encode(&mine, b""), ANN, &cache, &os),
        AdminReply::NeedsAuth,
        "a password given a moment ago does not change it again"
    );
    let bobs = req(AdminAction::UserPassword, &["bob", "n3w-passw0rd"]);
    assert_eq!(
        handle(&encode(&bobs, b"wrong"), BOB, &cache, &os),
        AdminReply::Denied,
        "not an administrator, but his own: the password decides"
    );
    assert_eq!(
        handle(&encode(&mine, b"good"), BOB, &cache, &os),
        AdminReply::NotAdmin
    );
}

/// REGRESSION GUARD for live-4 F1 (QA-STORE-42 family): the bare `pacman -Syu` of rel/0.12.1 is
/// already gone from this tree (UPD-restore); this keeps it from coming back. Update Now's steps run under wol-admind's cleared
/// `PATH=/usr/bin:/usr/sbin`, where a bare `pacman` is the real one and not the sealed
/// system's wrapper in /usr/local/bin. The slot update names pacman by its path, in the slot
/// and in place, and never starts a bare one.
#[test]
fn update_now_never_starts_a_bare_pacman() {
    assert!(!wol_core::privileged::PATH.contains("/usr/local/bin"));
    let slot = slot_os();
    let up = encode(&req(AdminAction::PkgUpgrade, &["gimp"]), b"");
    assert_eq!(handle(&up, ROOT, &Shared::default(), &slot), AdminReply::Ok);
    let one = Fake {
        files: vec![(wol_core::update::SINGLE_COPY.into(), String::new())],
        ..Fake::default()
    };
    assert_eq!(handle(&up, ROOT, &Shared::default(), &one), AdminReply::Ok);
    for os in [&slot, &one] {
        let ran = os.ran.borrow();
        let mut pacmans = 0;
        for argv in ran.iter() {
            assert_ne!(argv[0], "pacman", "{argv:?}");
            pacmans += argv
                .iter()
                .filter(|a| a.as_str() == wol_core::update::PACMAN)
                .count();
            assert!(
                !argv.iter().any(|a| a == "pacman"),
                "a bare pacman among {argv:?}"
            );
        }
        assert!(pacmans >= 1, "the update itself runs pacman: {ran:?}");
    }
}

/// Live-4 F1: the journal says why a step failed, not only that it did.
#[test]
fn a_failed_update_step_logs_what_it_said() {
    let os = Fake {
        fail_on: Some("mkinitcpio"),
        stderr: "error: failed to synchronize all databases (unable to lock database)\n",
        ..slot_os()
    };
    let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
    assert_eq!(
        handle(&up, ROOT, &Shared::default(), &os),
        AdminReply::Failed
    );
    assert!(
        os.log.borrow().iter().any(|l| l.contains(
            "failed: error: failed to synchronize all databases (unable to lock database); wolos-b is as it was"
        )),
        "{:?}",
        os.log.borrow()
    );
    let one = Fake {
        files: vec![(wol_core::update::SINGLE_COPY.into(), String::new())],
        fail_run: true,
        stderr: "error: no space left on device",
        ..Fake::default()
    };
    assert_eq!(
        handle(&up, ROOT, &Shared::default(), &one),
        AdminReply::Failed
    );
    assert!(
        one.log
            .borrow()
            .iter()
            .any(|l| l.contains("failed: error: no space left on device")),
        "{:?}",
        one.log.borrow()
    );
}

/// Low Power Mode probes the machine, saves the mode and switches power-profiles-daemon.
#[test]
fn low_power_mode_follows_the_probed_hardware() {
    let os = Fake {
        probes: vec![("nproc", "2\n"), ("powerprofilesctl get", "balanced\n")],
        ..Fake::default()
    };
    let low = encode(&req(AdminAction::PowerMode, &["low"]), b"");
    assert_eq!(handle(&low, ROOT, &Shared::default(), &os), AdminReply::Ok);
    let ran: Vec<String> = os.ran.borrow().iter().map(|a| a.join(" ")).collect();
    assert_eq!(ran, ["powerprofilesctl set power-saver"]);
    assert_eq!(
        os.wrote.borrow()[0],
        (
            "/etc/wolos/power.json".to_string(),
            "{\"mode\":\"low\"}\n".to_string()
        )
    );
    let probed: Vec<String> = os.captured.borrow().iter().map(|a| a.join(" ")).collect();
    assert!(
        probed.contains(&"ls /sys/class/power_supply".to_string()),
        "{probed:?}"
    );
    // Nothing to steer (VM: no power-profiles-daemon): refused, nothing runs.
    let vm = Fake::default();
    assert_eq!(
        handle(&low, ROOT, &Shared::default(), &vm),
        AdminReply::Failed
    );
    assert!(vm.ran.borrow().is_empty() && vm.wrote.borrow().is_empty());
}

/// Low Power Mode, Wi-Fi, Bluetooth and Sleep are like on a Mac: the person at the computer
/// (an account with a desktop session), no password. Accounts without a session, unknown peers
/// and every system-wide action still need an administrator.
#[test]
fn everyday_changes_need_no_password_at_the_computer() {
    let os = Fake {
        probes: vec![("nproc", "1\n"), ("powerprofilesctl get", "balanced\n")],
        sessions: vec![1001, 1002],
        ..Fake::default()
    };
    let low = encode(&req(AdminAction::PowerMode, &["low"]), b"");
    for (caller, want) in [
        (ANN, AdminReply::Ok),
        (BOB, AdminReply::Ok),
        (None, AdminReply::NotAdmin),
    ] {
        assert_eq!(
            handle(&low, caller, &Shared::default(), &os),
            want,
            "{caller:?}"
        );
    }
    let sleep = encode(&req(AdminAction::SessionPower, &["sleep"]), b"");
    assert_eq!(handle(&sleep, BOB, &Shared::default(), &os), AdminReply::Ok);
    assert_eq!(
        os.ran.borrow().last().unwrap().join(" "),
        "systemctl suspend"
    );
    assert_eq!(os.auths.get(), 0);
    assert!(
        os.log.borrow().iter().any(|l| l.contains("session-power")),
        "audited: {:?}",
        os.log.borrow()
    );
    let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
    assert_eq!(
        handle(&up, BOB, &Shared::default(), &os),
        AdminReply::NotAdmin
    );
    // Ann shutting down would end Bob's session: her password, like an administrator change.
    let off = encode(&req(AdminAction::SessionPower, &["shutdown"]), b"");
    assert_eq!(
        handle(&off, ANN, &Shared::default(), &os),
        AdminReply::NeedsAuth
    );
    // Bob restarting would end Ann's session: refused, and the log says who is signed in
    // (the lock screen's Restart, audit6 S6-5), not "not an administrator".
    let restart = encode(&req(AdminAction::SessionPower, &["restart"]), b"");
    assert_eq!(
        handle(&restart, BOB, &Shared::default(), &os),
        AdminReply::NotAdmin
    );
    let last = os.log.borrow().last().cloned().unwrap_or_default();
    assert!(last.ends_with("refused, signed in: ann(1001)"), "{last}");
    // No desktop session (a remote shell): no everyday shortcut.
    let away = Fake::default();
    let join = encode(&req(AdminAction::WifiPower, &["wlan0", "on"]), b"");
    assert_eq!(
        handle(&join, BOB, &Shared::default(), &away),
        AdminReply::NotAdmin
    );
}

#[test]
fn passwd_lookup_and_atomic_write() {
    assert_eq!(
        user_of("ann:*:1001:20::/h:/bin/sh\n", 1001),
        Some(("ann".into(), 20))
    );
    assert_eq!(user_of("ann:*:1001:20::/h:/bin/sh\n", 5), None);
    let p = std::env::temp_dir().join(format!("wol-admind-{}.txt", std::process::id()));
    atomic_write(&p, 0o600, b"a").unwrap();
    atomic_write(&p, 0o600, b"b").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"b");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::remove_file(p).unwrap();
}

const SEED: [u8; 32] = [7; 32];
const INFO: &str = r#"{"id":"org.wolos.Fixture","version":"1.0","package":"wol-fixture","targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/fixture"}]}"#;

/// A `.wol` file on disk, signed with `seed` when given.
fn wol_file(name: &str, seed: Option<&[u8; 32]>, payload: &[u8]) -> String {
    use wol_core::package::{manifest, sign, ustar};
    let man = manifest(&[("Info.json", INFO.as_bytes()), ("payload.pkg", payload)]);
    let sig = seed.map(|s| sign(s, man.as_bytes()));
    let mut m: Vec<(&str, &[u8])> = vec![
        ("Info.json", INFO.as_bytes()),
        ("payload.pkg", payload),
        ("SHA256", man.as_bytes()),
    ];
    if let Some(sig) = &sig {
        m.push(("SHA256.sig", sig));
    }
    let p = std::env::temp_dir().join(format!("wol-admind-{}-{name}.wol", std::process::id()));
    std::fs::write(&p, ustar(&m)).unwrap();
    p.to_string_lossy().into_owned()
}

fn pkg_os() -> Fake {
    Fake {
        keys: vec![wol_core::package::public_of(&SEED)],
        pkg: vec![("payload.pkg", "wol-fixture 1.0\n".into())],
        ..Fake::default()
    }
}

fn ran(os: &Fake) -> Vec<String> {
    let stage = os.stage.borrow().clone();
    let stage = stage.as_ref().map(|p| p.to_string_lossy().into_owned());
    os.ran
        .borrow()
        .iter()
        .map(|a| {
            let l = a.join(" ");
            stage
                .as_ref()
                .map_or(l.clone(), |s| l.replace(s, "<stage>"))
        })
        .collect()
}

/// App Store installs publish their percentage; other verbs run plainly.
#[test]
fn store_installs_publish_progress() {
    let os = Fake::default();
    for (a, t) in [
        (AdminAction::PkgInstall, "gimp"),
        (AdminAction::FlatpakInstall, "org.gimp.GIMP"),
        (AdminAction::PkgRemove, "gimp"),
    ] {
        let r = req(a, &[t]);
        assert_eq!(
            handle(&encode(&r, b""), ROOT, &Shared::default(), &os),
            AdminReply::Ok
        );
    }
    assert_eq!(
        *os.progress.borrow(),
        [
            "/run/wolos/progress/gimp",
            "/run/wolos/progress/org.gimp.GIMP"
        ]
    );
    assert_eq!(os.ran.borrow().len(), 3);
}

/// A package that installed leaves modules-install's gave-up list (Settings > Modules Retry);
/// a failed install, another name or no list leaves it alone.
#[test]
fn an_installed_package_leaves_the_gave_up_list() {
    let file = wol_core::modules::GAVE_UP_FILE;
    let list = "m dev\np gst-libav\nm ai-local\n";
    let os = Fake {
        files: vec![(file.into(), list.into())],
        ..Fake::default()
    };
    let go = |os: &Fake, name: &str| {
        let r = req(AdminAction::PkgInstall, &[name]);
        handle(&encode(&r, b""), ROOT, &Shared::default(), os)
    };
    assert_eq!(go(&os, "wolos-module-dev"), AdminReply::Ok);
    assert_eq!(go(&os, "gst-libav"), AdminReply::Ok);
    assert_eq!(go(&os, "lame"), AdminReply::Ok);
    let wrote = os.wrote.borrow().clone();
    assert_eq!(
        wrote,
        [
            (file.to_string(), "p gst-libav\nm ai-local\n".to_string()),
            (file.to_string(), "m dev\nm ai-local\n".to_string()),
        ],
        "one write per name on the list; `lame` was not there"
    );
    let failing = Fake {
        fail_run: true,
        files: vec![(file.into(), list.into())],
        ..Fake::default()
    };
    assert_eq!(go(&failing, "wolos-module-dev"), AdminReply::Failed);
    assert!(
        failing.wrote.borrow().is_empty(),
        "a failed install keeps its line"
    );
    assert_eq!(go(&Fake::default(), "wolos-module-dev"), AdminReply::Ok);
}

/// Signed `.wol`: verified on the daemon's own copy, then `wol pkg -- -U` (deps from the repos).
#[test]
fn wol_install_signed() {
    let os = pkg_os();
    let f = wol_file("signed", Some(&SEED), b"payload");
    let r = req(AdminAction::WolInstall, &[&f, "signed"]);
    assert_eq!(
        handle(&encode(&r, b""), ROOT, &Shared::default(), &os),
        AdminReply::Ok
    );
    assert_eq!(ran(&os), ["wol pkg -- -U --noconfirm <stage>/payload.pkg"]);
    let stage = os.stage.borrow().clone().unwrap();
    assert!(!stage.exists(), "staging removed");
    // Same version installed: nothing runs.
    let os = Fake {
        pkg: [
            pkg_os().pkg,
            vec![("-Q wol-fixture", "wol-fixture 1.0\n".into())],
        ]
        .concat(),
        ..pkg_os()
    };
    assert_eq!(
        handle(&encode(&r, b""), ROOT, &Shared::default(), &os),
        AdminReply::Ok
    );
    assert!(ran(&os).is_empty());
}

/// Refusals: nothing runs, the log says why.
#[test]
fn wol_install_refusals() {
    let signed = wol_file("r-signed", Some(&SEED), b"payload");
    let unsigned = wol_file("r-unsigned", None, b"payload");
    let other_key = wol_file("r-other", Some(&[9; 32]), b"payload");
    let tampered = wol_file("r-tampered", Some(&SEED), b"payload");
    let mut b = std::fs::read(&tampered).unwrap();
    // The payload bytes (the header name is "payload.pkg\0").
    let at = b.windows(8).position(|w| w == b"payload\0").unwrap();
    b[at] = b'P';
    std::fs::write(&tampered, b).unwrap();
    let mismatch = Fake {
        pkg: vec![("payload.pkg", "other 1.0\n".into())],
        ..pkg_os()
    };
    let cases: [(&str, &str, Fake, &str); 4] = [
        (&unsigned, "signed", pkg_os(), "unsigned"),
        (&other_key, "signed", pkg_os(), "trusted key"),
        (&tampered, "unsigned", pkg_os(), "hash mismatch"),
        (&signed, "signed", mismatch, "does not match"),
    ];
    for (f, trust, os, why) in cases {
        let r = req(AdminAction::WolInstall, &[f, trust]);
        assert_eq!(
            handle(&encode(&r, b""), ROOT, &Shared::default(), &os),
            AdminReply::Failed,
            "{why}"
        );
        assert!(os.ran.borrow().is_empty(), "{why}: {:?}", os.ran.borrow());
        assert!(
            os.log.borrow().iter().any(|l| l.contains(why)),
            "{why}: {:?}",
            os.log.borrow()
        );
    }
}

/// Unsigned needs the password every time, even with a cached authorization.
#[test]
fn opening_the_aur_always_checks_the_password() {
    let os = Fake::default();
    let cache = Shared::default();
    let ann = Some(Caller {
        uid: 1001,
        session: 7,
    });
    let open = |pw: &[u8]| {
        encode(
            &AdminRequest {
                action: AdminAction::AurOpen,
                args: vec![],
            },
            pw,
        )
    };
    assert_eq!(handle(&open(b""), ann, &cache, &os), AdminReply::NeedsAuth);
    assert_eq!(handle(&open(b"bad"), ann, &cache, &os), AdminReply::Denied);
    assert_eq!(handle(&open(b"good"), ann, &cache, &os), AdminReply::Ok);
    // A password given a moment ago does not open it again.
    assert_eq!(handle(&open(b""), ann, &cache, &os), AdminReply::NeedsAuth);
    let bob = Some(Caller {
        uid: 1002,
        session: 8,
    });
    assert_eq!(
        handle(&open(b"good"), bob, &cache, &os),
        AdminReply::NotAdmin
    );
    assert!(os.ran.borrow().is_empty());
}

#[test]
fn wol_install_unsigned_needs_a_fresh_password() {
    let os = pkg_os();
    let cache = Shared::default();
    cache.cached().cache.grant(1001, 7, os.now());
    let f = wol_file("fresh", None, b"payload");
    let r = req(AdminAction::WolInstall, &[&f, "unsigned"]);
    assert_eq!(
        handle(&encode(&r, b""), ANN, &cache, &os),
        AdminReply::NeedsAuth
    );
    assert_eq!(
        handle(&encode(&r, b"good"), ANN, &cache, &os),
        AdminReply::Ok
    );
    assert_eq!(
        ran(&os).last().unwrap(),
        "wol pkg -- -U --noconfirm <stage>/payload.pkg"
    );
    // Signed installs use the cache as before.
    let s = wol_file("fresh-signed", Some(&SEED), b"payload");
    let r = req(AdminAction::WolInstall, &[&s, "signed"]);
    assert_eq!(handle(&encode(&r, b""), ANN, &cache, &os), AdminReply::Ok);
}

#[test]
fn wol_remove_reads_the_install_record() {
    let os = Fake {
        files: vec![(
            "/usr/share/wolos/packages/org.wolos.Fixture/Info.json".into(),
            INFO.into(),
        )],
        ..Fake::default()
    };
    let r = req(AdminAction::WolRemove, &["org.wolos.Fixture"]);
    assert_eq!(
        handle(&encode(&r, b""), ROOT, &Shared::default(), &os),
        AdminReply::Ok
    );
    assert_eq!(ran(&os), ["wol pkg -- -Rns --noconfirm wol-fixture"]);
    let r = req(AdminAction::WolRemove, &["org.wolos.Missing"]);
    let os = Fake::default();
    assert_eq!(
        handle(&encode(&r, b""), ROOT, &Shared::default(), &os),
        AdminReply::Failed
    );
    assert!(os.ran.borrow().is_empty());
}

#[test]
fn caller_may_read_table() {
    // (file uid, mode, caller uid, ok)
    for (owner, mode, caller, ok) in [
        (1001, 0o600, Some(1001), true),
        (1002, 0o600, Some(1001), false),
        (1002, 0o644, Some(1001), true),
        (0, 0o600, Some(0), true),
        (0, 0o600, None, false),
        (1001, 0o044, Some(1001), true),
        (1001, 0o040, Some(1001), false),
    ] {
        assert_eq!(
            caller_may_read(owner, mode, caller),
            ok,
            "{owner} {mode:o} {caller:?}"
        );
    }
}

/// The walk is of the opened file's real directories: a private directory above it shows.
#[test]
fn dirs_above_lists_the_opened_files_directories() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let base = std::env::temp_dir().join(format!("wol-admind-reach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let private = base.join("private");
    std::fs::create_dir_all(&private).unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = private.join("x.pkg");
    std::fs::write(&file, b"x").unwrap();
    let link = base.join("link.pkg");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    let me = std::fs::metadata(&private).unwrap().uid();
    let dirs = dirs_above(&std::fs::File::open(&link).unwrap()).unwrap();
    assert_eq!(dirs[0], (me, std::fs::metadata(&private).unwrap().mode()));
    assert_eq!(
        dirs.len(),
        private.canonicalize().unwrap().ancestors().count()
    );
    assert!(!caller_may_reach(&dirs, Some(me.wrapping_add(1).max(1))));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_file_inside_a_private_directory_is_not_reachable() {
    let root = (0, 0o755);
    let home = (0, 0o755);
    let mine = (1001, 0o700);
    let theirs = (1002, 0o700);
    assert!(caller_may_reach(&[mine, home, root], Some(1001)));
    assert!(!caller_may_reach(&[theirs, home, root], Some(1001)));
    assert!(!caller_may_reach(&[(1002, 0o750), home, root], Some(1001)));
    assert!(caller_may_reach(&[theirs, home, root], Some(0)));
    assert!(!caller_may_reach(&[root], None));
    for (p, ok) in [
        ("/home/a/x.wol", true),
        ("/home/a/../b/x.wol", false),
        ("home/a/x.wol", false),
        ("/home//a", false),
        ("/home/./a", false),
    ] {
        assert_eq!(is_plain_absolute(p), ok, "{p}");
    }
}

/// Audit S4-7: the deadline is for the whole request, not per read: a client that trickles
/// a byte at a time is cut off at the deadline, an idle one too.
#[test]
fn a_trickling_client_is_cut_off_at_the_deadline() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let (srv, cli) = UnixStream::pair().unwrap();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let trickle = std::thread::spawn(move || {
        // One byte every 50 ms until told to stop (the channel's timeout is the pause).
        while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
            stop_rx.recv_timeout(std::time::Duration::from_millis(50))
        {
            if (&cli).write_all(b"x").is_err() {
                break;
            }
        }
    });
    let mut buf = vec![0u8; 1024];
    let t = std::time::Instant::now();
    let n = read_request(&srv, &mut buf, std::time::Duration::from_millis(300));
    let took = t.elapsed();
    assert!(took < std::time::Duration::from_millis(900), "{took:?}");
    assert!(n < 20, "{n} bytes");
    let _ = stop_tx.send(());
    trickle.join().unwrap();
    let (srv, _idle) = UnixStream::pair().unwrap();
    let t = std::time::Instant::now();
    assert_eq!(
        read_request(&srv, &mut buf, std::time::Duration::from_millis(200)),
        0
    );
    assert!(t.elapsed() < std::time::Duration::from_millis(800));
}

/// Audit S4-7: a password check that stalls (PAM) is killed at its timeout and counts as
/// not accepted; the daemon is not frozen.
#[test]
fn a_stalled_password_check_is_a_no_not_a_hang() {
    let mut slow = std::process::Command::new("sleep");
    slow.arg("10");
    let t = std::time::Instant::now();
    assert!(!check_password(
        slow,
        b"pw",
        std::time::Duration::from_millis(300)
    ));
    assert!(
        t.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        t.elapsed()
    );
    let ok = std::process::Command::new("true");
    assert!(check_password(ok, b"pw", AUTH_TIMEOUT));
    let no = std::process::Command::new("false");
    assert!(!check_password(no, b"pw", AUTH_TIMEOUT));
}

/// Audit S4-7 and ADR 0033 on wol-admind: an idle client does not hold the daemon (a second
/// client's `revoke` is answered within a second), and a request sent without answering the
/// hello gets no answer at all.
#[test]
fn an_idle_client_does_not_hold_wol_admind() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    static OS: RealOs = RealOs;
    let dir = std::env::temp_dir().join(format!("wol-admind-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("admin.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || serve(&listener, &OS));
    let _idle = UnixStream::connect(&path).unwrap();
    let req = shared::admin::AdminRequest {
        action: AdminAction::Revoke,
        args: Vec::new(),
    };
    let t = std::time::Instant::now();
    let mut s = shared::jobs::connect(&path).unwrap();
    s.write_all(&shared::admin::encode(&req, b"")).unwrap();
    s.shutdown(std::net::Shutdown::Write).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut line = String::new();
    s.read_to_string(&mut line).unwrap();
    assert_eq!(AdminReply::parse(&line), AdminReply::Ok, "{line}");
    assert!(
        t.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        t.elapsed()
    );
    // No hello answered: the request is never read and nothing comes back.
    let mut raw = UnixStream::connect(&path).unwrap();
    raw.write_all(&shared::admin::encode(&req, b"")).unwrap();
    raw.shutdown(std::net::Shutdown::Write).unwrap();
    raw.set_read_timeout(Some(std::time::Duration::from_secs(8)))
        .unwrap();
    let mut got = String::new();
    let _ = raw.read_to_string(&mut got);
    assert!(!got.contains("ok"), "{got}");
    let _ = std::fs::remove_dir_all(&dir);
}
/// QA-100 STORE-4: Update Now starts nothing while another update holds the update lock; the
/// sentence says why, and the log names the lock. Without the lock it runs as before.
#[test]
fn update_now_refused_while_locked() {
    let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
    let held = Fake {
        locked: true,
        ..slot_os()
    };
    let mut said = None;
    let r = handle_said(&up, ROOT, &Shared::default(), &held, &mut said);
    assert_eq!(r, AdminReply::Failed);
    assert_eq!(said.as_deref(), Some(UPDATE_RUNNING));
    assert!(held.ran.borrow().is_empty(), "no step ran: {:?}", held.ran);
    assert!(
        held.log.borrow().iter().any(|l| l.contains(UPDATE_LOCK)),
        "{:?}",
        held.log.borrow()
    );
    // The one-copy path is held to it as well.
    let one = Fake {
        locked: true,
        files: vec![(wol_core::update::SINGLE_COPY.into(), String::new())],
        ..Fake::default()
    };
    assert_eq!(
        handle(&up, ROOT, &Shared::default(), &one),
        AdminReply::Failed
    );
    assert!(one.ran.borrow().is_empty());
    let free = slot_os();
    assert_eq!(handle(&up, ROOT, &Shared::default(), &free), AdminReply::Ok);
}

/// The probe behind it, on a real file: a held flock reads as held, a free or missing one not; a
/// file root did not make for this (another owner, group or other bits) is ignored; and the probe
/// holds no lock of its own, so an exclusive `flock -n` right after it succeeds.
#[test]
fn the_update_lock_probe_sees_a_held_flock() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("wol-admind-lock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("update.lock");
    let me = shared::sys::euid();
    assert!(!lock_held_by_owner(&path, me), "missing file");
    let f = std::fs::File::create(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(!lock_held_by_owner(&path, me), "free");
    f.lock().unwrap();
    assert!(lock_held_by_owner(&path, me), "held by another description");
    assert!(
        !lock_held_by_owner(&path, me + 1),
        "not made by the expected owner"
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(
        !lock_held_by_owner(&path, me),
        "other-writable: anyone could have made it"
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert!(!lock_held_by_owner(&path, me), "group bits");
    drop(f);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    // Closing releases the lock, but a child another test forked in this very moment holds a copy of
    // the descriptor until its exec: wait that out (it is microseconds) instead of racing it.
    let released = (0..2_000_000).any(|_| {
        std::thread::yield_now();
        !lock_held_by_owner(&path, me)
    });
    assert!(released, "released on close");
    // The probe took nothing: a builder's non-blocking exclusive lock works at once.
    let builder = std::fs::File::open(&path).unwrap();
    assert!(builder.try_lock().is_ok());
    assert!(!lock_held(&path), "root-owned is required in the daemon");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn proc_locks_lines_are_read() {
    let dev = (8u64 << 8) | 1; // 08:01
    let text = "1: POSIX  ADVISORY  WRITE 99 08:01:77 0 EOF\n\
                2: FLOCK  ADVISORY  WRITE 4242 08:01:131074 0 EOF\n\
                3: FLOCK  ADVISORY  READ 4243 08:01:555 0 EOF\n\
                4: -> FLOCK  ADVISORY  WRITE 4244 08:01:556 0 EOF\n";
    assert!(flock_write_held(text, dev, 131074));
    assert!(!flock_write_held(text, dev, 77), "POSIX is not flock");
    assert!(
        !flock_write_held(text, dev, 555),
        "a shared lock does not stop a builder"
    );
    assert!(!flock_write_held(text, dev, 556), "a waiter holds nothing");
    assert!(
        !flock_write_held(text, (9 << 8) | 1, 131074),
        "other device"
    );
}

/// QA-100 LIVE3-19, DEV-4: a failed step's last stderr line reaches the log and the reply
/// (`failed <sentence>`), not "The change failed" alone.
#[test]
fn capture_returns_last_stderr_line() {
    assert_eq!(reason_of(""), None);
    assert_eq!(reason_of("\n  \n"), None);
    assert_eq!(
        reason_of("modprobe: warning\nmodprobe: ERROR: No such device\n\n").as_deref(),
        Some("modprobe: ERROR: No such device")
    );
    assert_eq!(
        reason_of("a\u{1b}[31mb\n").as_deref(),
        Some("a[31mb"),
        "controls dropped"
    );
    let long = reason_of(&"é".repeat(300)).unwrap();
    assert!(
        shared::admin::failed_line(&format!("The change failed: {long}")).len()
            <= "failed ".len() + shared::admin::MAX_FAILURE_MESSAGE
    );
    assert!(long.starts_with('é'));
    // Through a request: lpadmin failing says why.
    let os = Fake {
        fail_run: true,
        stderr: "lpadmin: Bad device-uri \"ipp://x\".\n",
        ..Fake::default()
    };
    let add = encode(
        &req(AdminAction::PrinterAdd, &["HP", "ipp://10.0.0.5/ipp/print"]),
        b"",
    );
    let mut said = None;
    let r = handle_said(&add, ROOT, &Shared::default(), &os, &mut said);
    assert_eq!(r, AdminReply::Failed);
    let line = reply_line(r, said.as_deref());
    assert_eq!(
        shared::admin::failure_message(&line).as_deref(),
        Some("The change failed: lpadmin: Bad device-uri \"ipp://x\".")
    );
    assert!(
        os.log
            .borrow()
            .iter()
            .any(|l| l.ends_with("failed: lpadmin: Bad device-uri \"ipp://x\".")),
        "{:?}",
        os.log.borrow()
    );
    // A step that said nothing leaves the plain line.
    let quiet = Fake {
        fail_run: true,
        ..Fake::default()
    };
    let mut said = None;
    handle_said(&add, ROOT, &Shared::default(), &quiet, &mut said);
    assert_eq!(said, None);
    // The real thing: a child that fails leaves its last line for the daemon.
    let real = RealOs;
    let argv = [
        "/bin/sh",
        "-c",
        "echo first >&2; echo 'no adapter' >&2; exit 1",
    ]
    .map(String::from);
    assert!(!real.run(&argv, false));
    assert_eq!(reason_of(&real.last_error()).as_deref(), Some("no adapter"));
}

/// QA-100 SHADOW-20: an unreadable account list is logged as that.
#[test]
fn an_empty_account_list_is_logged_as_unreadable() {
    struct NoPasswd(Fake);
    impl AdminOs for NoPasswd {
        fn passwd(&self) -> String {
            String::new()
        }
        fn group(&self) -> String {
            self.0.group()
        }
        fn authenticate(&self, u: &str, p: &[u8]) -> bool {
            self.0.authenticate(u, p)
        }
        fn run(&self, a: &[String], l: bool) -> bool {
            self.0.run(a, l)
        }
        fn run_input(&self, a: &[String], i: &str) -> bool {
            self.0.run_input(a, i)
        }
        fn run_progress(&self, a: &[String], k: wol_core::progress::Kind, p: &str) -> bool {
            self.0.run_progress(a, k, p)
        }
        fn write(&self, p: &str, m: u32, t: &str) -> bool {
            self.0.write(p, m, t)
        }
        fn read(&self, p: &str) -> Option<String> {
            self.0.read(p)
        }
        fn refuse_delay(&self) {}
        fn now(&self) -> u64 {
            100
        }
        fn log(&self, t: &str) {
            self.0.log(t)
        }
        fn sessions(&self) -> Vec<u32> {
            vec![]
        }
        fn capture(&self, a: &[String]) -> Option<String> {
            self.0.capture(a)
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
            vec![]
        }
        fn remove_dir(&self, _: &std::path::Path) {}
        fn package_facts(&self, _: &str) -> Result<crate::packages::PkgFacts, String> {
            Err(String::new())
        }
    }
    let os = NoPasswd(Fake::default());
    let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"good");
    assert_eq!(
        handle(&up, ANN, &Shared::default(), &os),
        AdminReply::NotAdmin
    );
    assert!(
        os.0.log
            .borrow()
            .iter()
            .any(|l| l.contains("/etc/passwd") && l.contains("unreadable")),
        "{:?}",
        os.0.log.borrow()
    );
}

/// QA-100 STORE-2: when the daily check holds a signed newer release, Update Now builds the new
/// slot on that release's snapshot (the repos' date steps), in the slot and in place; with a package
/// held back, or without a release, it stays on the pinned snapshot as before.
#[test]
fn trusted_base_adds_date_steps() {
    let dated = |os: &Fake| {
        let ran = os.ran.borrow();
        ran.iter().any(|argv| {
            argv.first().is_some_and(|a| a == "sed")
                && argv.iter().any(|a| a.contains("2026/11/02"))
        })
    };
    let up = |holds: &[&str]| encode(&req(AdminAction::PkgUpgrade, holds), b"");
    // In the other slot.
    let slot = Fake {
        release: Some(("2026.11", "2026/11/02")),
        ..slot_os()
    };
    assert_eq!(
        handle(&up(&[]), ROOT, &Shared::default(), &slot),
        AdminReply::Ok
    );
    assert!(
        dated(&slot),
        "the slot's repos move to the release: {:?}",
        slot.ran
    );
    assert!(
        slot.log
            .borrow()
            .iter()
            .any(|l| l.contains("WolOS 2026.11")),
        "{:?}",
        slot.log.borrow()
    );
    // Same slot, same steps otherwise: still a snapshot of the running root first, pacman inside.
    let plain = slot_os();
    handle(&up(&[]), ROOT, &Shared::default(), &plain);
    assert!(!dated(&plain), "no release: the pinned snapshot");
    assert_eq!(
        slot.ran.borrow().len(),
        plain.ran.borrow().len() + 2,
        "two sed steps more, nothing else"
    );
    // A held package keeps the release for later.
    let held = Fake {
        release: Some(("2026.11", "2026/11/02")),
        ..slot_os()
    };
    handle(&up(&["gimp"]), ROOT, &Shared::default(), &held);
    assert!(!dated(&held));
    assert!(held.log.borrow().iter().any(|l| l.contains("held back")));
    // One copy of WolOS: in place.
    let one = Fake {
        release: Some(("2026.11", "2026/11/02")),
        files: vec![(wol_core::update::SINGLE_COPY.into(), String::new())],
        ..Fake::default()
    };
    assert_eq!(
        handle(&up(&[]), ROOT, &Shared::default(), &one),
        AdminReply::Ok
    );
    assert!(dated(&one), "{:?}", one.ran);
}

/// QA-100: the error-report switch and the metered-download switch go through wol-admind as
/// administrator changes: refused for a standard account, done for an administrator, and they
/// write the files the sender and the download script read.
#[test]
fn reports_and_metered_switches_are_administrator_changes() {
    let os = Fake::default();
    let cache = Shared::default();
    for (action, args, file, text) in [
        (
            AdminAction::Reports,
            vec!["on"],
            "/etc/wolos/reports.conf",
            "send=1\n",
        ),
        (
            AdminAction::Reports,
            vec!["off"],
            "/etc/wolos/reports.conf",
            "send=0\n",
        ),
    ] {
        let good = encode(&req(action, &args), b"good");
        assert_eq!(handle(&good, BOB, &cache, &os), AdminReply::NotAdmin);
        assert_eq!(handle(&good, ANN, &cache, &os), AdminReply::Ok);
        assert!(
            os.wrote.borrow().contains(&(file.into(), text.into())),
            "{:?}",
            os.wrote.borrow()
        );
    }
    assert_eq!(
        os.ran.borrow().last().map(|a| a.join(" ")).as_deref(),
        Some("systemctl disable --now wolos-report.timer")
    );
    let metered = encode(&req(AdminAction::AutoUpdate, &["metered", "on"]), b"good");
    assert_eq!(handle(&metered, ANN, &cache, &os), AdminReply::Ok);
    let wrote = os.wrote.borrow();
    let (path, text) = wrote.last().unwrap();
    assert_eq!(path, wol_core::update::auto::FILE);
    assert!(text.contains("metered=on"), "{text}");
}

/// QA-100 NET2: a join that fails (wrong password) deletes the profile it made, once, after the
/// failed step, and the reply still carries the step's own words; the person's saved profile stays.
#[test]
fn failed_join_cleanup_runs_after_the_failure() {
    let join = encode(
        &req(AdminAction::WifiJoin, &["wlan0", "Home", "password1"]),
        b"",
    );
    let fresh = Fake {
        fail_on: Some("connection up"),
        stderr: "Error: Secrets were required, but not provided.",
        probes: vec![(
            "nmcli -t -f NAME,TYPE connection show",
            "Wired connection 1:802-3-ethernet\n",
        )],
        ..Fake::default()
    };
    let mut said = None;
    let r = handle_said(&join, ROOT, &Shared::default(), &fresh, &mut said);
    assert_eq!(r, AdminReply::Failed);
    let ran: Vec<String> = fresh.ran.borrow().iter().map(|a| a.join(" ")).collect();
    assert_eq!(
        ran.iter()
            .filter(|l| l.starts_with("nmcli connection delete id Home"))
            .count(),
        1,
        "{ran:?}"
    );
    assert!(
        ran.last().unwrap().starts_with("nmcli connection delete"),
        "after the steps: {ran:?}"
    );
    assert_eq!(
        said.as_deref(),
        Some("The change failed: Error: Secrets were required, but not provided.")
    );
    // The profile was the person's before: it stays.
    let saved = Fake {
        fail_on: Some("connection up"),
        probes: vec![(
            "nmcli -t -f NAME,TYPE connection show",
            "Home:802-11-wireless\n",
        )],
        ..Fake::default()
    };
    handle(&join, ROOT, &Shared::default(), &saved);
    assert!(
        saved
            .ran
            .borrow()
            .iter()
            .all(|a| !a.join(" ").contains("connection delete"))
    );
    // A join that worked deletes nothing.
    let good = Fake {
        probes: vec![("nmcli -t -f NAME,TYPE connection show", "")],
        ..Fake::default()
    };
    assert_eq!(
        handle(&join, ROOT, &Shared::default(), &good),
        AdminReply::Ok
    );
    assert!(
        good.ran
            .borrow()
            .iter()
            .all(|a| !a.join(" ").contains("delete"))
    );
}

/// Requests from many threads at once (api-security 1+2 and its review).
mod concurrency;
/// Update Now: the restore point, one copy, a failed slot (UPD-restore).
mod update_now;

/// QA-100 STORE-42: wol-admind's children run with `PATH=/usr/bin:/usr/sbin`, where a bare
/// `pacman` is the stock one whose database is on the sealed /usr ("unable to lock database").
/// No administrator plan (refresh, install, remove, Update Now in place or in a slot) starts one:
/// pacman is reached through `wol pkg --` (the layer or the pending slot) or by its absolute path.
#[test]
fn no_admin_plan_names_bare_pacman() {
    use wol_core::admin::{AdminStep, plan_with};
    assert!(!wol_core::privileged::PATH.contains("/usr/local/bin"));
    let tries: [&[&str]; 6] = [
        &[],
        &["gimp"],
        &["steam"],
        &["codecs-kit"],
        &["wolos-b"],
        &["gimp", "x"],
    ];
    let mut planned = 0;
    for action in AdminAction::ALL {
        for args in tries {
            let r = req(action, args);
            let Ok(steps) = plan_with(&r, &|_| None) else {
                continue;
            };
            for s in steps {
                let argv = match s {
                    AdminStep::Run(a) | AdminStep::RunInput { argv: a, .. } => a,
                    AdminStep::Write { .. } => continue,
                };
                planned += 1;
                assert!(
                    !argv.iter().any(|a| a == "pacman"),
                    "{action:?} {args:?}: {argv:?}"
                );
            }
        }
    }
    assert!(planned > 20, "{planned}");
    // Update Now itself, in a slot and in place.
    for os in [
        slot_os(),
        Fake {
            files: vec![(wol_core::update::SINGLE_COPY.into(), String::new())],
            ..Fake::default()
        },
    ] {
        let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
        handle(&up, ROOT, &Shared::default(), &os);
        assert!(
            os.ran
                .borrow()
                .iter()
                .all(|a| !a.iter().any(|x| x == "pacman")),
            "{:?}",
            os.ran.borrow()
        );
    }
}
