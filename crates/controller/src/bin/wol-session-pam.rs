// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-session-pam USER PROGRAM [NAME=VALUE]: wol-login's session runner (root). Opens USER's PAM
//! session (pam_systemd: the logind session on seat0, /run/user, user@.service with PipeWire and
//! the portals), starts PROGRAM (wol-session only, `controller::login::session_exec`) directly as
//! USER with the session's environment, never through USER's login shell (QA-SHADOW-43: a pwsh
//! account could not start a session), closes the session when it ends and exits with its status.
//! Exit 77: not root, no such user or another program; 70: PAM refused.
#![allow(unsafe_code)]

use controller::login::{PASSWD_FILE, SESSION_PAM_SERVICE, find, session_exec};
use controller::pam;
use std::ffi::CString;
use std::os::unix::process::CommandExt;
use std::process::{Command, exit};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // SAFETY: plain getter.
    if args.len() < 2 || unsafe { libc::geteuid() } != 0 {
        exit(77);
    }
    let Some((program, vars)) = session_exec(&args[1..]) else {
        exit(77);
    };
    let passwd = std::fs::read_to_string(PASSWD_FILE).unwrap_or_default();
    let Some(u) = find(&passwd, &args[0]) else {
        exit(77);
    };
    let Ok(name) = CString::new(u.name.clone()) else {
        exit(77);
    };
    let Some(session) = pam::open_session(
        SESSION_PAM_SERVICE,
        &name,
        c"tty1",
        &[
            "XDG_SESSION_TYPE=wayland",
            "XDG_SESSION_CLASS=user",
            "XDG_SESSION_DESKTOP=WolOS",
            "XDG_SEAT=seat0",
            "XDG_VTNR=1",
        ],
    ) else {
        eprintln!("wol-session-pam: {}: PAM refused the session", u.name);
        exit(70);
    };
    let (uid, gid) = (u.uid, u.gid);
    let mut cmd = Command::new(&program);
    cmd.env_clear()
        .envs(session.env.iter().filter_map(|e| e.split_once('=')))
        .envs(vars)
        .env("HOME", &u.home)
        .env("USER", &u.name)
        .env("LOGNAME", &u.name)
        .env("SHELL", &u.shell)
        .env("PATH", "/usr/local/bin:/usr/bin")
        .current_dir(&u.home);
    // SAFETY: only async-signal-safe calls between fork and exec; `name` outlives the spawn.
    unsafe {
        cmd.pre_exec(move || {
            if libc::initgroups(name.as_ptr(), gid) != 0
                || libc::setgid(gid) != 0
                || libc::setuid(uid) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let status = cmd.spawn().and_then(|mut c| c.wait());
    drop(session);
    exit(status.ok().and_then(|s| s.code()).unwrap_or(1));
}
