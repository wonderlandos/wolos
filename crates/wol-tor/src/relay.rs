// SPDX-License-Identifier: GPL-2.0-or-later
//! The SOCKS5 relay inside the browser's network namespace (ADR 0053 D2): it listens on that
//! namespace's loopback and forwards each `CONNECT` to tor's Unix socket, with the stream's
//! isolation credential ([`crate::isolation`]).
//!
//! The kill switch is structural: the only thing this file ever connects to is `tor_socks`. When
//! tor is not running, or not bootstrapped within [`Relay::ready_wait`], the browser gets SOCKS
//! reply 1 (general failure) and nothing else happens; there is no fallback to report, because
//! none exists. Internal targets (loopback, private, link-local, `.local`, `.localhost`) are
//! refused before tor is asked, so a page cannot use the relay to reach the machine.
use crate::isolation::Isolator;
use crate::socks::{self, Host, Reply, SocksError, Target};
use std::fmt;
use std::io::{self, Write};
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Time a browser has to finish the SOCKS handshake.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Time tor has to answer the relay's handshake (a circuit may need to be built).
pub const TOR_TIMEOUT: Duration = Duration::from_secs(60);

/// Whether tor has finished bootstrapping. Set by the control-port watcher.
#[derive(Debug, Default)]
pub struct Ready {
    state: Mutex<bool>,
    cv: Condvar,
}

impl Ready {
    /// Marks tor bootstrapped (or not, after it stopped).
    pub fn set(&self, ready: bool) {
        *self.state.lock().unwrap_or_else(|p| p.into_inner()) = ready;
        self.cv.notify_all();
    }

    /// Waits until ready or until `timeout`; true when ready.
    pub fn wait(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut g = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while !*g {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            g = match self.cv.wait_timeout(g, deadline - now) {
                Ok((g, _)) => g,
                Err(p) => p.into_inner().0,
            };
        }
        true
    }
}

/// What happened to one browser connection.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Carried: tor accepted the `CONNECT`, the bytes were relayed until one side closed.
    Relayed,
    /// Refused with this reply before any data moved.
    Refused(Reply),
}

/// A relay error that ended a connection without a reply (the peer went away, or spoke
/// something other than SOCKS5).
#[derive(Debug)]
pub struct RelayError(pub SocksError);

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for RelayError {}

/// A target the relay will ask tor for. Tor itself refuses internal addresses too
/// (`ClientRejectInternalAddresses`); this check keeps them from reaching tor at all.
pub fn allowed(t: &Target) -> bool {
    match &t.host {
        Host::V4(a) => v4_public(*a),
        Host::V6(a) => match a.to_ipv4_mapped() {
            Some(v4) => v4_public(v4),
            None => v6_public(a),
        },
        Host::Domain(d) => {
            let d = d.trim_end_matches('.');
            !(d == "localhost"
                || d.ends_with(".localhost")
                || d.ends_with(".local")
                || d.ends_with(".internal")
                || d.parse::<std::net::IpAddr>().is_ok())
        }
    }
}

fn v4_public(a: Ipv4Addr) -> bool {
    let o = a.octets();
    let cgnat = o[0] == 100 && (o[1] & 0xc0) == 64;
    !(a.is_private()
        || a.is_loopback()
        || a.is_link_local()
        || a.is_unspecified()
        || a.is_broadcast()
        || a.is_multicast()
        || cgnat
        || o[0] == 0)
}

fn v6_public(a: &Ipv6Addr) -> bool {
    let s = a.segments();
    let unique_local = (s[0] & 0xfe00) == 0xfc00;
    let link_local = (s[0] & 0xffc0) == 0xfe80;
    !(a.is_loopback() || a.is_unspecified() || a.is_multicast() || unique_local || link_local)
}

/// One group's relay.
#[derive(Debug)]
pub struct Relay {
    /// tor's SOCKS Unix socket: the only place this relay connects to.
    pub tor_socks: PathBuf,
    /// The group's credentials.
    pub isolator: Arc<Isolator>,
    /// tor's bootstrap state.
    pub ready: Arc<Ready>,
    /// How long a connect waits for tor to finish bootstrapping before it fails.
    pub ready_wait: Duration,
}

fn refuse(c: &mut TcpStream, r: Reply) -> Outcome {
    let _ = c.write_all(&socks::reply_bytes(r));
    let _ = c.shutdown(Shutdown::Both);
    Outcome::Refused(r)
}

impl Relay {
    /// Accepts browser connections forever, one named thread each.
    pub fn serve(self: Arc<Self>, listener: TcpListener) -> io::Result<()> {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            let me = Arc::clone(&self);
            thread::Builder::new()
                .name("wol-tor-stream".into())
                .spawn(move || {
                    let _ = me.handle(conn);
                })?;
        }
        Ok(())
    }

    /// One browser connection, from greeting to close.
    pub fn handle(&self, mut c: TcpStream) -> Result<Outcome, RelayError> {
        c.set_read_timeout(Some(HANDSHAKE_TIMEOUT))
            .map_err(|e| RelayError(e.into()))?;
        let method = socks::server_greeting(&mut c).map_err(RelayError)?;
        let offered = if method == socks::USER_PASS {
            Some(socks::server_userpass(&mut c).map_err(RelayError)?)
        } else {
            None
        };
        let target = match socks::server_request(&mut c) {
            Ok(t) => t,
            Err(e) => match e.reply() {
                Some(r) => return Ok(refuse(&mut c, r)),
                None => return Err(RelayError(e)),
            },
        };
        if !allowed(&target) {
            return Ok(refuse(&mut c, Reply::NotAllowed));
        }
        if !self.ready.wait(self.ready_wait) {
            return Ok(refuse(&mut c, Reply::GeneralFailure));
        }
        let Ok(mut tor) = UnixStream::connect(&self.tor_socks) else {
            return Ok(refuse(&mut c, Reply::GeneralFailure));
        };
        if tor.set_read_timeout(Some(TOR_TIMEOUT)).is_err() {
            return Ok(refuse(&mut c, Reply::GeneralFailure));
        }
        let (user, pass) = self.isolator.credential(offered.as_ref(), &target);
        match socks::client_connect(&mut tor, &user, &pass, &target) {
            Ok(Reply::Succeeded) => {}
            Ok(r) => return Ok(refuse(&mut c, r)),
            Err(_) => return Ok(refuse(&mut c, Reply::GeneralFailure)),
        }
        c.write_all(&socks::reply_bytes(Reply::Succeeded))
            .map_err(|e| RelayError(e.into()))?;
        // Data phase: a page may keep a socket open and quiet for long (web sockets).
        let _ = c.set_read_timeout(None);
        let _ = tor.set_read_timeout(None);
        splice(c, tor).map_err(|e| RelayError(e.into()))?;
        Ok(Outcome::Relayed)
    }
}

/// Copies both ways until either side closes, then closes both.
fn splice(c: TcpStream, tor: UnixStream) -> io::Result<()> {
    let (mut c_read, mut tor_write) = (c.try_clone()?, tor.try_clone()?);
    let up = thread::Builder::new()
        .name("wol-tor-up".into())
        .spawn(move || {
            let _ = io::copy(&mut c_read, &mut tor_write);
            let _ = tor_write.shutdown(Shutdown::Write);
        })?;
    let (mut tor_read, mut c_write) = (tor, c);
    let _ = io::copy(&mut tor_read, &mut c_write);
    let _ = c_write.shutdown(Shutdown::Write);
    let _ = up.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(h: &str) -> Target {
        Target {
            host: Host::Domain(h.into()),
            port: 80,
        }
    }

    #[test]
    fn internal_targets_never_reach_tor() {
        for t in [
            "localhost",
            "a.localhost",
            "printer.local",
            "router.internal",
            "127.0.0.1",
            "::1",
        ] {
            assert!(!allowed(&d(t)), "{t}");
        }
        for a in [
            [127, 0, 0, 1],
            [10, 1, 2, 3],
            [192, 168, 1, 1],
            [172, 16, 0, 1],
            [169, 254, 1, 1],
            [100, 64, 0, 1],
            [0, 0, 0, 0],
            [224, 0, 0, 1],
        ] {
            assert!(
                !allowed(&Target {
                    host: Host::V4(Ipv4Addr::from(a)),
                    port: 80
                }),
                "{a:?}"
            );
        }
        for v6 in ["::1", "fe80::1", "fd00::1", "::ffff:192.168.0.1", "ff02::1"] {
            let a: Ipv6Addr = v6.parse().unwrap();
            assert!(
                !allowed(&Target {
                    host: Host::V6(a),
                    port: 80
                }),
                "{v6}"
            );
        }
    }

    #[test]
    fn public_targets_and_onions_are_allowed() {
        assert!(allowed(&d("example.org")));
        assert!(allowed(&d(
            "duckduckgogg42xjoc72x3sjasowoarfbgcmvfimaftt6twagswzczad.onion"
        )));
        assert!(allowed(&Target {
            host: Host::V4(Ipv4Addr::new(93, 184, 215, 14)),
            port: 443
        }));
        assert!(allowed(&Target {
            host: Host::V6("2606:4700::1".parse().unwrap()),
            port: 443
        }));
    }

    #[test]
    fn ready_wait_times_out_then_sees_ready() {
        let r = Ready::default();
        assert!(!r.wait(Duration::from_millis(5)));
        r.set(true);
        assert!(r.wait(Duration::from_millis(5)));
    }
}
