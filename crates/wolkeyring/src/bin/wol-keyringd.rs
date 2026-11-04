// SPDX-License-Identifier: BSD-2-Clause
//! `wol-keyringd`: the keyring daemon of one session (ADR 0054). It serves the fast path on
//! `$XDG_RUNTIME_DIR/wolos/keyring.sock` and the Secret Service subset on the session bus, locks
//! after the idle time and with the screen and sleep, and opens the keyring at sign-in with the
//! TPM device key when the keyring has one.
//!
//!   wol-keyringd [--name NAME] [--no-bus]
//!
//! `--no-bus` serves the socket only (tests, machines without a session bus).
use std::sync::Arc;
use std::time::Duration;
use wolkeyring::daemon::{self, Daemon};
use wolkeyring::{io, secret_service};

fn logind_watchers(d: &Arc<Daemon>) {
    use zbus::message::Type;
    let rules: Vec<(&'static str, &'static str, Option<String>, bool)> = vec![
        (
            "org.freedesktop.login1.Manager",
            "PrepareForSleep",
            None,
            true,
        ),
        (
            "org.freedesktop.login1.Session",
            "Lock",
            std::env::var("XDG_SESSION_ID")
                .ok()
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric()))
                .map(|s| {
                    // sd_bus_path_encode: only a leading digit needs escaping here (ids are alphanumeric).
                    let esc: String = s
                        .bytes()
                        .enumerate()
                        .map(|(i, b)| {
                            if i == 0 && b.is_ascii_digit() {
                                format!("_{b:02x}")
                            } else {
                                (b as char).to_string()
                            }
                        })
                        .collect();
                    format!("/org/freedesktop/login1/session/{esc}")
                }),
            false,
        ),
    ];
    for (iface, member, path, sleep) in rules {
        if !sleep && path.is_none() {
            continue; // Without our own session id any session's Lock would lock this keyring.
        }
        let d = d.clone();
        std::thread::spawn(move || {
            let Ok(conn) = zbus::blocking::Connection::system() else {
                return;
            };
            let mut b = zbus::MatchRule::builder().msg_type(Type::Signal);
            let Ok(r) = (|| -> zbus::Result<zbus::MatchRule<'static>> {
                b = b.interface(iface)?.member(member)?;
                if let Some(p) = &path {
                    b = b.path(p.as_str())?;
                }
                Ok(b.build().to_owned())
            })() else {
                return;
            };
            let Ok(it) = zbus::blocking::MessageIterator::for_match_rule(r, &conn, None) else {
                return;
            };
            for m in it.flatten() {
                if sleep {
                    // PrepareForSleep(true) comes before the machine sleeps.
                    if m.body().deserialize::<bool>().unwrap_or(false) {
                        d.lock_event(true);
                    }
                } else {
                    d.lock_event(false);
                }
            }
        });
    }
}

/// No core file, and not dumpable: another process of the same user cannot read this one's
/// memory through ptrace or `/proc/<pid>/mem`, and a crash writes nothing (ADR 0054 section 4).
/// The library has no `unsafe`; the two calls live here, in the program.
fn keep_memory_private() {
    let zero = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: both calls take plain integers or a pointer to a live struct, and only change this
    // process's own limits.
    let (a, b) = unsafe {
        (
            libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0),
            libc::setrlimit(libc::RLIMIT_CORE, &zero),
        )
    };
    if a != 0 || b != 0 {
        eprintln!("wol-keyringd: could not make its memory private; stopping");
        std::process::exit(1);
    }
}

fn main() {
    keep_memory_private();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let no_bus = args
        .iter()
        .position(|a| a == "--no-bus")
        .map(|i| args.remove(i))
        .is_some();
    let name = match args.as_slice() {
        [] => "login".to_string(),
        [f, n] if f == "--name" => n.clone(),
        _ => {
            eprintln!("usage: wol-keyringd [--name NAME] [--no-bus]");
            std::process::exit(2);
        }
    };
    let d = Arc::new(Daemon::new(&name));
    let sock = match io::socket_path().and_then(|p| daemon::bind(&p).map(|l| (p, l))) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("wol-keyringd: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = d.unlock_at_start() {
        eprintln!("wol-keyringd: starts locked ({e})");
    }
    {
        let d = d.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(5));
                d.tick();
            }
        });
    }
    let listener = sock.1;
    if no_bus {
        daemon::serve(d, listener);
        return;
    }
    {
        let d = d.clone();
        std::thread::spawn(move || daemon::serve(d, listener));
    }
    logind_watchers(&d);
    // The Secret Service; the connection's own executor thread serves it while this one waits.
    match zbus::block_on(secret_service::serve(d.clone())) {
        Ok(_conn) => loop {
            std::thread::park();
        },
        Err(e) => {
            // Another owner of org.freedesktop.secrets (gnome-keyring, before migration): keep
            // the fast path, say so, and do not pretend.
            eprintln!("wol-keyringd: no Secret Service on the bus ({e}); fast path only");
            loop {
                std::thread::park();
            }
        }
    }
}
