// SPDX-License-Identifier: BSD-2-Clause
//! `atlas-fed`: the Atlas browser's Federate helper (protocol and limits: the library).
//!
//! ```text
//! atlas-fed [--socket PATH] [--idle-exit SECONDS] [--overlay DIR]
//!     serve fed:// loads on PATH (default $XDG_RUNTIME_DIR/atlas/fed.sock); exits after SECONDS
//!     with no load (default 600, 0 never); a second helper on the same socket exits at once (0)
//! atlas-fed --get URL [--subresource] [--overlay DIR]
//!     answer one load here: status and type on stderr, the body on stdout
//! atlas-fed --get-socket URL [--socket PATH]
//!     ask a running helper, as the browser does
//! ```
//! `--overlay DIR` serves the files under `DIR/v1/...` in place of the network's answers (a
//! tampering distributor, for tests and demonstrations): they are checked like the network's.
#![forbid(unsafe_code)]

use atlas_fed::{Handler, Limits, Overlay};
use federate::{Load, Resolver};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

/// How long `--get-socket` waits for the helper's reply (a load of a block over the network).
const GET_WAIT: Duration = Duration::from_secs(120);

const USAGE: &str = "usage: atlas-fed [--socket PATH] [--idle-exit SECONDS] [--overlay DIR]\n       \
atlas-fed --get URL [--subresource] [--overlay DIR]\n       atlas-fed --get-socket URL [--socket PATH]";

#[derive(Default)]
struct Args {
    socket: Option<PathBuf>,
    idle: Option<u64>,
    overlay: Option<PathBuf>,
    get: Option<String>,
    get_socket: Option<String>,
    subresource: bool,
}

fn parse(argv: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--socket" => a.socket = Some(PathBuf::from(value()?)),
            "--idle-exit" => {
                let v = value()?;
                a.idle = Some(
                    v.parse()
                        .map_err(|_| format!("--idle-exit {v}: not a number of seconds"))?,
                );
            }
            "--overlay" => a.overlay = Some(PathBuf::from(value()?)),
            "--get" => a.get = Some(value()?),
            "--get-socket" => a.get_socket = Some(value()?),
            "--subresource" => a.subresource = true,
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

fn resolver(overlay: Option<PathBuf>) -> Resolver {
    let door: Box<dyn federate::Source> = Box::new(federate::Https::new(federate::BOOTSTRAP));
    let source: Box<dyn federate::Source> = match overlay {
        Some(dir) => Box::new(Overlay::new(dir, door)),
        None => door,
    };
    // The newest root zone version seen survives a restart (ADR 0052, "Rollback and time").
    Resolver::new(source, federate::ROOT_KEY).with_state(federate::resolve::default_state(&env))
}

fn socket_path(a: &Args) -> Result<PathBuf, String> {
    a.socket
        .clone()
        .or_else(|| atlas_fed::default_socket(&env))
        .ok_or_else(|| {
            "no --socket and no XDG_RUNTIME_DIR: there is no place for the socket".into()
        })
}

fn print(reply: &federate::Reply) -> ExitCode {
    eprintln!("{} {}", reply.status, reply.mime);
    let mut out = std::io::stdout().lock();
    if out
        .write_all(&reply.body)
        .and_then(|()| out.flush())
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    if reply.status == 200 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let a = match parse(&argv) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("atlas-fed: {e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let kind = if a.subresource {
        Load::Subresource
    } else {
        Load::Document
    };
    if let Some(url) = &a.get {
        return print(&resolver(a.overlay.clone()).serve_load(url, kind));
    }
    let path = match socket_path(&a) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("atlas-fed: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(url) = &a.get_socket {
        let asked = UnixStream::connect(&path).and_then(|mut s| {
            s.set_read_timeout(Some(GET_WAIT))?;
            s.write_all(atlas_fed::request_line(url, kind).as_bytes())?;
            atlas_fed::read_reply(&mut s, federate::source::MAX_BLOCK)
        });
        return match asked {
            Ok(r) => print(&r),
            Err(e) => {
                eprintln!("atlas-fed: {} did not answer ({e})", path.display());
                ExitCode::FAILURE
            }
        };
    }
    let bound = match atlas_fed::bind(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            eprintln!("atlas-fed: {e}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("atlas-fed: {} could not be opened ({e})", path.display());
            return ExitCode::FAILURE;
        }
    };
    let r = Arc::new(resolver(a.overlay.clone()));
    let handler: Handler = Arc::new(move |url, kind| r.serve_load(url, kind));
    let idle = match a.idle.unwrap_or(600) {
        0 => None,
        s => Some(Duration::from_secs(s)),
    };
    let limits = Limits {
        idle,
        ..Limits::default()
    };
    let served = atlas_fed::run(bound.listener, handler, limits);
    // Idle: the socket goes with the process (the lock is still ours here, so no other helper
    // has bound it); the next fed:// load starts a new helper.
    let _ = std::fs::remove_file(&bound.path);
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("atlas-fed: serving {} stopped ({e})", bound.path.display());
            ExitCode::FAILURE
        }
    }
}
