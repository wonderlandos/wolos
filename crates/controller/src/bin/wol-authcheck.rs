//! wol-authcheck: setuid-root helper for the lock screen (IaraOS ADR 0023).
//! Reads a password from stdin, checks it with PAM service `wol-lock` for the user of the real uid,
//! and answers with the exit code only (0 ok, 1 denied, 2 error). No environment, no output.
//! `--admin <user>` (real uid 0 only, for wol-admind): check `<user>` with PAM `wol-admin`; else exit 77.
use controller::authcheck::*;
use controller::pam;
use std::mem::ManuallyDrop;
use std::os::fd::FromRawFd;
use std::time::Duration;

const SERVICE: &std::ffi::CStr = c"wol-lock";
const ADMIN_SERVICE: &std::ffi::CStr = c"wol-admin";
const FAIL_DELAY: Duration = Duration::from_secs(2);

fn run(buf: &mut [u8; MAX_PASSWORD + 2], mode: Mode) -> Auth {
    for (k, _) in std::env::vars_os() {
        // SAFETY: single-threaded at this point.
        unsafe { std::env::remove_var(k) }
    }
    let no_core = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: valid resource id and pointer.
    unsafe { libc::setrlimit(libc::RLIMIT_CORE, &no_core) };
    let (service, user) = match mode {
        Mode::Lock => match pam::real_user() {
            Some(u) => (SERVICE, u),
            None => return Auth::Error,
        },
        Mode::Admin(name) => match std::ffi::CString::new(name) {
            Ok(u) => (ADMIN_SERVICE, u),
            Err(_) => return Auth::Error,
        },
    };
    // Unbuffered fd 0, so the only copy of the password is `buf`.
    // SAFETY: fd 0 stays open for the process lifetime; ManuallyDrop never closes it.
    let mut stdin = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(0) });
    let Ok(n) = read_password(&mut *stdin, buf) else {
        return Auth::Error;
    };
    pam::authenticate(service, &user, &buf[..n])
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // SAFETY: getuid has no preconditions.
    let mode = match mode(&args, unsafe { libc::getuid() }) {
        Ok(m) => m,
        Err(code) => {
            std::thread::sleep(FAIL_DELAY);
            std::process::exit(code);
        }
    };
    let mut buf = [0u8; MAX_PASSWORD + 2];
    let result = run(&mut buf, mode);
    wipe(&mut buf);
    if result != Auth::Ok {
        std::thread::sleep(FAIL_DELAY);
    }
    std::process::exit(exit_code(&result));
}
