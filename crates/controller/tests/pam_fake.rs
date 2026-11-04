// SPDX-License-Identifier: GPL-2.0-or-later
//! The PAM stacks WolOS ships (desktop/overlay/etc/pam.d), run for real through
//! `controller::pam::authenticate_in` against fake fixtures (tests/fixtures/pam): a temporary
//! configuration directory holds the shipped file with the parts that need root pointed at the
//! temporary directory (pam_faillock's tally, pam_nologin's file) and the Arch stacks it includes
//! (system-auth, system-local-login) replaced by permit-only ones. pam_unix checks the account
//! running the tests through unix_chkpwd, so a wrong password is a real pam_unix refusal, with
//! its own fail delay when the stack asks for one.
use controller::authcheck::{Auth, FAIL_DELAY, Mode, delay_for};
use controller::pam::{authenticate_in, real_user};
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SHIPPED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../desktop/overlay/etc/pam.d"
);
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pam");
const WRONG: &[u8] = b"not-the-password-of-this-account";

/// A configuration directory under the system temp dir, removed on drop.
struct Confdir(PathBuf);

// The temporary configuration directory goes with the test that made it.
impl Drop for Confdir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Confdir {
    fn c(&self) -> CString {
        CString::new(self.0.to_str().unwrap()).unwrap()
    }
    fn tally(&self) -> PathBuf {
        self.0.join("faillock")
    }
    fn nologin(&self) -> PathBuf {
        self.0.join("nologin")
    }
}

/// The shipped `service` stack under a fresh confdir. `unix_ok`: pam_unix becomes pam_permit (the
/// right password, which a test cannot know for the account running it).
fn stack(service: &str, unix_ok: bool) -> Confdir {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("wol-pam-fake-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("faillock")).unwrap();
    let d = Confdir(dir);
    for f in ["system-auth", "system-local-login"] {
        std::fs::copy(Path::new(FIXTURES).join(f), d.0.join(f)).unwrap();
    }
    let shipped = std::fs::read_to_string(Path::new(SHIPPED).join(service)).unwrap();
    let tally = format!("pam_faillock.so dir={}", d.tally().display());
    let nologin = format!("pam_nologin.so file={}", d.nologin().display());
    let text: String = shipped
        .lines()
        .map(|l| {
            let l = l
                .replace("pam_faillock.so", &tally)
                .replace("pam_nologin.so", &nologin);
            if unix_ok {
                l.replace("pam_unix.so", "pam_permit.so")
            } else {
                l
            }
        })
        .map(|l| l + "\n")
        .collect();
    std::fs::write(d.0.join(service), text).unwrap();
    d
}

fn me() -> CString {
    real_user().expect("the account running the tests has a passwd entry")
}

/// The wait after PAM refused, by the program that asked (ADR 0023/0024): wol-authcheck at the lock
/// screen, wol-login's reply at the login window, wol-admind's refusal (wol-authcheck --admin
/// leaves it to wol-admind) for Settings' Authenticate sheet.
fn helper_wait(service: &str) -> Duration {
    match service {
        "wol-lock" => {
            if delay_for(&Auth::Denied, &Mode::Lock) {
                FAIL_DELAY
            } else {
                Duration::ZERO
            }
        }
        "wol-login" => FAIL_DELAY,
        "wol-admin" => {
            let own = delay_for(&Auth::Denied, &Mode::Admin("x".into()));
            Duration::from_millis(wol_core::admin::REFUSAL_DELAY_MS)
                + if own { FAIL_DELAY } else { Duration::ZERO }
        }
        _ => unreachable!(),
    }
}

/// QA-SHADOW-35: one refused password costs the one fixed 2 s, not pam_unix's and pam_faillock's
/// own ~2 s on top (3.83 s measured at the lock screen, ~6 s in Settings).
#[test]
fn refusal_under_2_5s() {
    let user = me();
    for service in ["wol-lock", "wol-login", "wol-admin"] {
        let d = stack(service, false);
        let svc = CString::new(service).unwrap();
        for attempt in 1..=3 {
            let t = Instant::now();
            let got = authenticate_in(Some(&d.c()), &svc, &user, WRONG);
            let pam = t.elapsed();
            assert_eq!(got, Auth::Denied, "{service} attempt {attempt}");
            let total = pam + helper_wait(service);
            assert!(
                total < Duration::from_millis(2500),
                "{service} attempt {attempt}: PAM {pam:?} + helper {:?} = {total:?}",
                helper_wait(service)
            );
        }
    }
}

/// QA-SHADOW-18: a stack PAM cannot start (here: no stack and no `other` at all, PAM_ABORT) is an
/// error ('could not check'), never a wrong password, and is not delayed (QA-SHADOW-36).
#[test]
fn pam_start_failure_is_error() {
    let dir = CString::new(Path::new(FIXTURES).join("no-stack").to_str().unwrap()).unwrap();
    let t = Instant::now();
    let got = authenticate_in(Some(&dir), c"wol-lock", &me(), WRONG);
    assert_eq!(got, Auth::Error);
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "{:?}",
        t.elapsed()
    );
    assert!(!delay_for(&got, &Mode::Lock));
}

/// QA-FILES-9: a refusal that is not about the password never reads as 'Incorrect password'. The
/// login window lists exactly the accounts `shared::login::login_names` accepts (the shell rule
/// lives there, one rule for both sides), so the stack has no pam_shells of its own: an account
/// whose shell is missing from /etc/shells was listed and then refused as a wrong password. The
/// other such check, /etc/nologin, belongs to the account phase, where a refusal is an error.
#[test]
fn missing_shell_sentence() {
    let shipped = std::fs::read_to_string(Path::new(SHIPPED).join("wol-login")).unwrap();
    let auth_modules: Vec<&str> = shipped
        .lines()
        .filter(|l| l.split_whitespace().next() == Some("auth"))
        .collect();
    for m in ["pam_shells.so", "pam_nologin.so"] {
        assert!(
            !auth_modules.iter().any(|l| l.contains(m)),
            "wol-login auth runs {m}: its refusal is a wrong password"
        );
    }
    // /etc/nologin present, right password: an error, not 'Incorrect password'.
    let d = stack("wol-login", true);
    std::fs::write(d.nologin(), "The computer is being updated.\n").unwrap();
    let got = authenticate_in(Some(&d.c()), c"wol-login", &me(), b"right");
    assert_eq!(got, Auth::Error);
    // And without it the same stack lets the account in.
    std::fs::remove_file(d.nologin()).unwrap();
    assert_eq!(
        authenticate_in(Some(&d.c()), c"wol-login", &me(), b"right"),
        Auth::Ok
    );
}
