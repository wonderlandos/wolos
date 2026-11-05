// SPDX-License-Identifier: GPL-2.0-or-later
//! atlas-bridge: started by `atlas-bridge.socket` (systemd user unit) when Atlas first connects;
//! exits by itself when idle. `--socket <path>` binds a socket by hand for development.
#![forbid(unsafe_code)]
use atlas::server::{Config, IDLE, serve};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let listener = match (args.next().as_deref(), args.next()) {
        (Some("--socket"), Some(p)) => {
            atlas::activation::bind(p.as_ref()).map_err(|e| e.to_string())
        }
        (None, _) => {
            let var = |k| std::env::var(k).ok();
            atlas::activation::from_systemd(
                var("LISTEN_PID").as_deref(),
                var("LISTEN_FDS").as_deref(),
            )
            .ok_or_else(|| "start me through atlas-bridge.socket, or with --socket <path>".into())
        }
        _ => Err("usage: atlas-bridge [--socket <path>]".into()),
    };
    let listener = match listener {
        Ok(l) => l,
        Err(m) => {
            eprintln!("atlas-bridge: {m}");
            return ExitCode::from(2);
        }
    };
    let browser = PathBuf::from(shared::atlas::BROWSER);
    let config = Config {
        browser: std::fs::canonicalize(&browser).unwrap_or(browser),
        wold: Arc::new(atlas::wold::Socket::from_setting(std::env::var_os(
            "WOL_SOCKET",
        ))),
        opener: atlas::verbs::OPENER.iter().map(|s| s.to_string()).collect(),
        idle: IDLE,
        // The journal collects stderr.
        log: Arc::new(|m| eprintln!("atlas-bridge: {m}")),
    };
    match serve(listener, config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("atlas-bridge: {e}");
            ExitCode::FAILURE
        }
    }
}
