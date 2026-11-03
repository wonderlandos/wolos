// SPDX-License-Identifier: BSD-2-Clause
//! The one display connection the native windows of a process share (each window has its own
//! event queue on it), whether the native backend may be chosen, and the session facts the
//! library does not read from the environment itself (`RR-027`): the locale for compose and dead
//! keys, and a startup activation token. `ffi/engine.rs` hands them over at engine creation.

use crate::copy;
use crate::error::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use wayland_client::Connection;

/// The shared connection.
pub struct Shared {
    pub conn: Connection,
}

impl Shared {
    /// A connection over an already connected socket (an in-process compositor in tests), not
    /// shared with anything else.
    pub fn from_socket(stream: std::os::unix::net::UnixStream) -> Result<Arc<Shared>> {
        let conn =
            Connection::from_socket(stream).map_err(|_| Error::unavailable(copy::NO_DISPLAY))?;
        Ok(Arc::new(Shared { conn }))
    }
}

static SHARED: Mutex<Option<Arc<Shared>>> = Mutex::new(None);
static OPT_IN: AtomicBool = AtomicBool::new(false);

/// What the session says, handed in by the C entry point.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    /// `LC_ALL`, `LC_CTYPE` or `LANG`, whichever is set first (compose table).
    pub locale: String,
    /// A token from the launcher (`XDG_ACTIVATION_TOKEN`) to raise the first window with.
    pub activation_token: Option<String>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

pub fn set_session(s: Session) {
    if let Ok(mut g) = SESSION.lock() {
        *g = Some(s);
    }
}

/// The session, with the locale defaulting to `C.UTF-8`.
pub fn session() -> Session {
    let s = SESSION
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default();
    Session {
        locale: if s.locale.is_empty() {
            "C.UTF-8".into()
        } else {
            s.locale
        },
        activation_token: s.activation_token,
    }
}

/// Take the startup token (it raises one window, once).
pub fn take_activation_token() -> Option<String> {
    SESSION
        .lock()
        .ok()
        .and_then(|mut g| g.as_mut().and_then(|s| s.activation_token.take()))
}

/// Let `window::choose` pick the native backend (until the CR3x gate makes it the default, the
/// Kit backend is chosen unless this is on).
pub fn set_opt_in(on: bool) {
    OPT_IN.store(on, Ordering::Relaxed);
}

pub fn opted_in() -> bool {
    OPT_IN.load(Ordering::Relaxed)
}

/// The shared connection, connecting on first use (`WAYLAND_DISPLAY`, read by the protocol
/// library itself). Fails with one sentence when there is no display.
pub fn shared() -> Result<Arc<Shared>> {
    let mut g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = g.as_ref() {
        return Ok(s.clone());
    }
    let conn = Connection::connect_to_env().map_err(|_| Error::unavailable(copy::NO_DISPLAY))?;
    let s = Arc::new(Shared { conn });
    *g = Some(s.clone());
    Ok(s)
}

/// Use this connection for the native windows from now on (tests: an in-process compositor).
pub fn use_socket(stream: std::os::unix::net::UnixStream) -> Result<Arc<Shared>> {
    let s = Shared::from_socket(stream)?;
    let mut g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some(s.clone());
    Ok(s)
}

/// Whether native windows can be opened: opted in and a display answers.
pub fn available() -> bool {
    opted_in() && shared().is_ok()
}
