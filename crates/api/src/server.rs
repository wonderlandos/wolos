// SPDX-License-Identifier: GPL-2.0-or-later
//! Socket glue for wold: one request line per connection; `subscribe` keeps the connection.
//! Invariants (ADR 0020, audit A2/A9/A10): identity is captured at accept (`peer.rs`), the request
//! line is bounded (`shared::portal::MAX_LINE`, [`CONNECTION_QUOTA`]) and must arrive within
//! [`CLIENT_TIMEOUT`]; a peer that exceeds a bound is dropped.
use crate::bus::Bus;
use crate::control::{JobTable, handle};
use crate::packages;
use crate::peer::PeerIdentity;
use crate::portal::{self, Grants};
use shared::bounded::BoundedLineReader;
use shared::events::Event;
use shared::jobs::{JobRequest, Reply};
use shared::sys::{FdReader, send_with_fd};
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

/// Bytes one control connection may send before it is dropped: one request line plus slack for a
/// hold (`hold_until_closed`).
pub const CONNECTION_QUOTA: u64 = shared::portal::MAX_LINE as u64 * 2;

/// Seconds a client may take to send its request line, and the write timeout for the answer.
pub const CLIENT_TIMEOUT: std::time::Duration = shared::bounded::CLIENT_TIMEOUT;

/// Write an error reply; the peer may already be gone.
fn refuse(stream: &UnixStream, why: &str) {
    let mut w = stream;
    let _ = writeln!(w, "{}", Reply::of(Some(why.into())));
}

/// Serve one control connection. `peer` was captured at accept ([`PeerIdentity::capture`]); grants
/// are decided on it only while the peer still runs the executable it ran then.
pub fn serve(
    stream: UnixStream,
    peer: Option<PeerIdentity>,
    jobs: &dyn JobTable,
    bus: &Bus<UnixStream>,
    grants: &dyn Grants,
    state: &portal::State,
) {
    let uid = peer.as_ref().map(|p| p.uid);
    let who = peer.as_ref().map(|p| (p.uid, p.pid));
    let caller = || peer.as_ref().and_then(PeerIdentity::caller);
    let _ = shared::bounded::limit_socket(&stream, CLIENT_TIMEOUT);
    // ADR 0033: greet first; the client's first line must echo the greeting, so bytes written
    // before this connection was accepted (by a process that has exec()ed since) never count, and
    // every byte of the echo must come from the process the kernel recorded at connect (audit S5:
    // not a child it forked before exec()ing a trusted binary).
    if let Err(e) = shared::jobs::greet(&stream) {
        return eprintln!("wold: dropped a client: {e}");
    }
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BoundedLineReader::new(
        FdReader::new(clone),
        shared::portal::MAX_LINE,
        CONNECTION_QUOTA,
    );
    let line = match reader.read_line_str() {
        Ok(Some(l)) => l,
        Ok(None) => return,
        Err(e) => return eprintln!("wold: dropped a client: {e}"),
    };
    let attached = reader.get_mut().fd.take();
    let req = JobRequest::parse(&line);
    // S4-26: grant-free requests too are refused from a peer whose known executable changed
    // since accept (exec after connect); an executable unknown on first sight is let through.
    if matches!(req, Some(JobRequest::Subscribe | JobRequest::Publish(_)))
        && peer.as_ref().is_some_and(PeerIdentity::changed)
    {
        // The refusal is the record: the caller hears why (no library logging, RR-004).
        return refuse(
            &stream,
            "permission denied: the caller changed since it connected",
        );
    }
    match &req {
        Some(JobRequest::Subscribe) => {
            // Publish writes are non-blocking: a subscriber that stops reading is dropped (bus.rs).
            if stream.set_nonblocking(true).is_ok() && !bus.subscribe(None, who, stream) {
                eprintln!("wold: refused a subscriber over its cap");
            }
        }
        Some(JobRequest::SubscribeAccount) => match (uid, caller()) {
            (Some(uid), Some(c)) if grants.shows_notifications(&c.app) => {
                let ok = stream.set_nonblocking(true).is_ok();
                if !(ok && bus.subscribe(Some(uid), who, stream)) {
                    eprintln!("wold: refused a subscriber over its cap");
                }
            }
            _ => refuse(&stream, "permission denied: notification shower only"),
        },
        Some(JobRequest::Publish(json)) => {
            // Everyone events from a client reach only its own uid (and root): bus.rs.
            if let (Some(e), Some(uid)) =
                (Event::parse(json).filter(Event::client_may_publish), uid)
            {
                bus.publish_from(Some(uid), &e);
            }
        }
        Some(JobRequest::Call(json)) => {
            let who = caller();
            let answer = if packages::is_package_call(json) {
                // ADR 0021: wopkg's system changes; the same consent grants, the real machine.
                let c = who.as_ref().map(|c| (c.uid, c.app.as_str()));
                packages::handle(
                    json,
                    c,
                    &|app, cap| {
                        let ok = grants.granted(app, cap);
                        if let (false, Some((uid, _))) = (ok, c) {
                            grants.refused(uid, app, cap);
                        }
                        ok
                    },
                    &packages::Host {
                        bus,
                        asks: &state.asks,
                    },
                )
            } else {
                let (answer, extra) =
                    portal::respond(json, who.as_ref(), grants, bus, state, attached);
                if let Some(held) = extra.hold {
                    let _ = (&stream).write_all(answer.to_line().as_bytes());
                    hold_until_closed(&stream);
                    let uid = held.uid;
                    drop(held);
                    state.holds.give(uid);
                    return;
                }
                if let Some(file) = extra.file {
                    let _ = send_with_fd(&stream, answer.to_line().as_bytes(), file.as_fd());
                    return;
                }
                answer
            };
            let mut w = &stream;
            let _ = w.write_all(answer.to_line().as_bytes());
        }
        _ => {
            let mut w = &stream;
            let _ = writeln!(w, "{}", handle(req.as_ref(), uid, jobs));
        }
    }
}

/// Until the caller closes the connection or dies. Whatever it sends is thrown away, up to a limit
/// that ends the hold: this thread does nothing else.
fn hold_until_closed(stream: &UnixStream) {
    let _ = stream.set_read_timeout(None);
    let _ = std::io::copy(&mut stream.take(4096), &mut std::io::sink());
}

/// Bind a socket path (stale file removed) with mode `mode`.
pub fn bind(path: &Path, mode: u32) -> std::io::Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_file(path);
    let l = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::jobs::JobStatus;
    use std::io::{BufRead, BufReader};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Empty;
    impl JobTable for Empty {
        fn list(&self) -> Vec<JobStatus> {
            vec![]
        }
        fn start(&self, _: &str) -> Option<Option<String>> {
            None
        }
        fn stop(&self, _: &str) -> Option<Option<String>> {
            None
        }
    }

    struct AllowAll;
    impl Grants for AllowAll {
        fn granted(&self, _: &str, _: &str) -> bool {
            true
        }
        fn shows_notifications(&self, _: &str) -> bool {
            true
        }
    }

    struct Strict;
    impl Grants for Strict {
        fn granted(&self, _: &str, _: &str) -> bool {
            true
        }
        fn shows_notifications(&self, _: &str) -> bool {
            false
        }
    }

    struct Deny;
    impl Grants for Deny {
        fn granted(&self, _: &str, _: &str) -> bool {
            false
        }
        fn shows_notifications(&self, _: &str) -> bool {
            false
        }
    }

    #[test]
    fn package_calls_reach_the_package_service_and_need_their_grant() {
        let bus = Bus::default();
        let (srv, mut cli) = UnixStream::pair().unwrap();
        let h = ask(
            &cli,
            b"call {\"call\":\"package.change\",\"verb\":\"install\",\"names\":[\"vim\"]}\n",
        );
        serve_me(srv, &Empty, &bus, &Deny, &portal::State::default());
        h.join().unwrap();
        let mut out = String::new();
        cli.read_to_string(&mut out).unwrap();
        assert!(out.contains("not allowed to install system apps"), "{out}");
    }

    /// Audit S1 regression: an account process binds a fake shell socket and would answer
    /// "allow-once". wold never dials it: the sheet goes to the shower subscription, and only the
    /// shower's `session.answer` (here: Deny) decides.
    #[test]
    fn a_package_sheet_is_answered_only_by_the_shower() {
        let dir = std::env::temp_dir().join(format!("wol-s1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = UnixListener::bind(dir.join(packages_shell_socket())).unwrap();
        fake.set_nonblocking(true).unwrap();
        let bus = Bus::default();
        let state = portal::State::default();
        let shower = call(
            "subscribe account
",
            &bus,
        );
        std::thread::scope(|t| {
            let (srv, app) = UnixStream::pair().unwrap();
            let h = ask(
                &app,
                b"call {\"call\":\"system.channel\",\"channel\":\"beta\"}
",
            );
            t.spawn(|| serve_me(srv, &Empty, &bus, &AllowAll, &state));
            h.join().unwrap();
            let mut line = String::new();
            BufReader::new(&shower).read_line(&mut line).unwrap();
            let Some(Event::SessionAsk {
                id,
                ask: shared::portal::Ask::Approval { approval },
                ..
            }) = Event::parse(&line)
            else {
                panic!("{line}")
            };
            assert_eq!(
                approval.helper,
                std::env::current_exe().unwrap().to_string_lossy(),
                "the sheet names the kernel's executable"
            );
            let deny = serde_json::to_string(&shared::shell::Answer::Deny).unwrap();
            let reply = format!(
                "call {{\"call\":\"session.answer\",\"id\":{id},\"data\":{deny}}}
"
            );
            let mut out = String::new();
            let (srv, desktop) = UnixStream::pair().unwrap();
            let d = ask(&desktop, reply.as_bytes());
            serve_me(srv, &Empty, &bus, &AllowAll, &state);
            d.join().unwrap();
            (&desktop).read_to_string(&mut out).unwrap();
            assert!(out.contains("\"ok\":true"), "{out}");
            let mut answer = String::new();
            BufReader::new(&app).read_line(&mut answer).unwrap();
            assert!(answer.contains("You said no"), "{answer}");
        });
        assert!(
            fake.accept().is_err(),
            "the fake shell socket was never dialled"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Where the shell's socket lives in the account's runtime directory.
    fn packages_shell_socket() -> &'static str {
        "wol-shell.sock"
    }

    /// Serve with the identity the kernel reports for the socket pair (this test process).
    fn serve_me(
        srv: UnixStream,
        jobs: &dyn JobTable,
        bus: &Bus<UnixStream>,
        grants: &dyn Grants,
        state: &portal::State,
    ) {
        let peer = PeerIdentity::capture(&srv);
        serve(srv, peer, jobs, bus, grants, state)
    }

    /// Client side on its own thread: answer wold's hello, then send `line` (the server runs on
    /// the test's thread).
    fn ask(cli: &UnixStream, line: &[u8]) -> std::thread::JoinHandle<()> {
        let (c, line) = (cli.try_clone().unwrap(), line.to_vec());
        std::thread::spawn(move || {
            shared::jobs::answer_hello(&c).unwrap();
            (&c).write_all(&line).unwrap();
        })
    }

    fn call(line: &str, bus: &Bus<UnixStream>) -> UnixStream {
        let (srv, cli) = UnixStream::pair().unwrap();
        let h = ask(&cli, line.as_bytes());
        serve_me(srv, &Empty, bus, &AllowAll, &portal::State::default());
        h.join().unwrap();
        cli
    }

    /// Audit S4-26: a peer whose executable was known at accept and is another now (it
    /// exec()ed) may neither publish nor subscribe; one whose executable was unknown on first
    /// sight still may (no grant involved).
    #[test]
    fn publish_and_subscribe_refuse_a_changed_identity_only() {
        let bus = Bus::default();
        let sub = call("subscribe\n", &bus);
        let e = Event::Theme {
            id: "wol-dark".into(),
        };
        let publish = |exe: Option<&str>| {
            let (srv, cli) = UnixStream::pair().unwrap();
            let h = ask(&cli, format!("publish {}", e.to_line()).as_bytes());
            let peer = PeerIdentity::captured_as(&srv, exe);
            serve(
                srv,
                peer,
                &Empty,
                &bus,
                &AllowAll,
                &portal::State::default(),
            );
            h.join().unwrap();
            cli
        };
        // Changed: refused with a reason, nothing reaches the subscriber.
        let mut answer = String::new();
        let mut cli = publish(Some("/usr/bin/not-what-runs"));
        cli.shutdown(std::net::Shutdown::Write).unwrap();
        cli.read_to_string(&mut answer).unwrap();
        assert!(answer.contains("changed since it connected"), "{answer}");
        // Unknown on first sight: published.
        drop(publish(None));
        let mut line = String::new();
        BufReader::new(&sub).read_line(&mut line).unwrap();
        assert_eq!(Event::parse(&line), Some(e.clone()));
        // A changed subscriber is not added.
        let before = bus.len();
        let (srv, cli) = UnixStream::pair().unwrap();
        let h = ask(&cli, b"subscribe\n");
        let peer = PeerIdentity::captured_as(&srv, Some("/usr/bin/not-what-runs"));
        serve(
            srv,
            peer,
            &Empty,
            &bus,
            &AllowAll,
            &portal::State::default(),
        );
        h.join().unwrap();
        assert_eq!(bus.len(), before);
    }

    #[test]
    fn list_subscribe_publish() {
        let bus = Bus::default();
        let mut out = String::new();
        call("list\n", &bus).read_to_string(&mut out).unwrap();
        assert_eq!(out, "[]\n");
        let sub = call("subscribe\n", &bus);
        assert_eq!(bus.len(), 1);
        let e = Event::Theme {
            id: "wol-dark".into(),
        };
        drop(call(&format!("publish {}", e.to_line()), &bus));
        let mut line = String::new();
        BufReader::new(sub).read_line(&mut line).unwrap();
        assert_eq!(Event::parse(&line), Some(e));
        drop(call("publish {bad}\n", &bus));
    }

    #[test]
    fn clients_cannot_forge_notifications_and_calls_answer() {
        let bus = Bus::default();
        let plain = call("subscribe\n", &bus);
        plain.set_nonblocking(true).unwrap();
        let sub = call("subscribe account\n", &bus);
        sub.set_nonblocking(true).unwrap();
        let forged = Event::Notification {
            uid: 1000,
            app: "/usr/bin/bank".into(),
            title: "t".into(),
            body: String::new(),
            id: 1,
            actions: Vec::new(),
            replace: false,
        };
        drop(call(&format!("publish {}", forged.to_line()), &bus));
        let mut r = BufReader::new(&sub);
        let mut line = String::new();
        assert!(r.read_line(&mut line).is_err() || line.is_empty(), "{line}");

        let mut out = String::new();
        call(
            "call {\"call\":\"notification.post\",\"title\":\"Hi\"}\n",
            &bus,
        )
        .read_to_string(&mut out)
        .unwrap();
        assert_eq!(out, "{\"ok\":true,\"id\":1}\n");
        sub.set_nonblocking(false).unwrap();
        r.read_line(&mut line).unwrap();
        let Some(Event::Notification { app, uid, .. }) = Event::parse(&line) else {
            panic!("{line}")
        };
        assert_eq!(uid, shared::sys::euid());
        assert_eq!(app, std::env::current_exe().unwrap().to_string_lossy());
        let mut other = String::new();
        let _ = BufReader::new(&plain).read_line(&mut other);
        assert_eq!(other, "", "plain subscribers never see notifications");
    }

    #[test]
    fn a_panel_hands_the_app_only_the_file_the_desktop_attached() {
        let bus = Bus::default();
        let state = portal::State::default();
        let sub = call("subscribe account\n", &bus);
        let path = std::env::temp_dir().join(format!("wol-panel-{}", std::process::id()));
        std::fs::write(&path, "picked").unwrap();
        std::thread::scope(|t| {
            let (srv, mut app) = UnixStream::pair().unwrap();
            t.spawn(|| serve_me(srv, &Empty, &bus, &AllowAll, &state));
            shared::jobs::answer_hello(&app).unwrap();
            app.write_all(b"call {\"call\":\"panel.open\",\"title\":\"Open\"}\n")
                .unwrap();

            let mut line = String::new();
            BufReader::new(&sub).read_line(&mut line).unwrap();
            let Some(Event::SessionAsk { id, ask, .. }) = Event::parse(&line) else {
                panic!("{line}")
            };
            assert_eq!(
                ask,
                shared::portal::Ask::Panel {
                    save: false,
                    title: "Open".into(),
                    name: String::new()
                }
            );
            let (srv, desktop) = UnixStream::pair().unwrap();
            let file = std::fs::File::open(&path).unwrap();
            let reply = format!(
                "call {{\"call\":\"session.answer\",\"id\":{id},\"data\":{{\"name\":\"a.txt\"}}}}\n"
            );
            t.spawn(move || {
                shared::jobs::answer_hello(&desktop).unwrap();
                send_with_fd(&desktop, reply.as_bytes(), file.as_fd()).unwrap();
            });
            serve_me(srv, &Empty, &bus, &AllowAll, &state);
            let mut r = BufReader::new(FdReader::new(app.try_clone().unwrap()));
            let mut got = String::new();
            r.read_line(&mut got).unwrap();
            assert_eq!(got, "{\"ok\":true,\"data\":{\"name\":\"a.txt\"}}\n");
            let mut text = String::new();
            std::fs::File::from(r.get_mut().fd.take().unwrap())
                .read_to_string(&mut text)
                .unwrap();
            assert_eq!(text, "picked");
        });
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_hold_ends_when_the_connection_closes() {
        struct Flag(Arc<AtomicBool>);
        impl Drop for Flag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        struct Fake(Arc<AtomicBool>);
        impl crate::facts::Inhibit for Fake {
            fn hold(&self, _: &str, _: &str) -> Option<Box<dyn Send>> {
                Some(Box::new(Flag(self.0.clone())))
            }
        }
        let let_go: Arc<AtomicBool> = Arc::default();
        let bus = Bus::default();
        let state = portal::State {
            inhibit: Box::new(Fake(let_go.clone())),
            ..portal::State::default()
        };
        std::thread::scope(|t| {
            let (srv, mut app) = UnixStream::pair().unwrap();
            let h = t.spawn(|| serve_me(srv, &Empty, &bus, &AllowAll, &state));
            shared::jobs::answer_hello(&app).unwrap();
            app.write_all(b"call {\"call\":\"power.keep-awake\",\"reason\":\"x\"}\n")
                .unwrap();
            let mut line = String::new();
            BufReader::new(&app).read_line(&mut line).unwrap();
            assert_eq!(line, "{\"ok\":true}\n");
            assert!(!let_go.load(Ordering::SeqCst), "held while connected");
            drop(app);
            h.join().unwrap();
        });
        assert!(let_go.load(Ordering::SeqCst));
    }

    #[test]
    fn only_the_shower_subscribes_to_account_events() {
        let bus = Bus::default();
        let (srv, mut cli) = UnixStream::pair().unwrap();
        let h = ask(&cli, b"subscribe account\n");
        serve_me(srv, &Empty, &bus, &Strict, &portal::State::default());
        h.join().unwrap();
        let mut out = String::new();
        cli.read_to_string(&mut out).unwrap();
        assert!(out.contains("permission denied"), "{out}");
        assert_eq!(bus.len(), 0);
    }

    /// Audit P6-1: a program that is a shower by its path alone is not one when it runs under
    /// `LD_PRELOAD` (exec()ed by a same-account process to load its own code); the same path
    /// without it subscribes.
    #[test]
    fn a_shower_binary_under_ld_preload_cannot_subscribe_to_the_account() {
        for (env, subscribes) in [(None, true), (Some("LD_PRELOAD"), false)] {
            let mut cmd = std::process::Command::new("/bin/sleep");
            // Not the test runner's environment: cargo sets LD_LIBRARY_PATH for tests.
            cmd.arg("10")
                .env_clear()
                .env("HOME", "/")
                .stderr(std::process::Stdio::null());
            if let Some(var) = env {
                cmd.env(var, "/nonexistent/evil.so");
            }
            let mut child = cmd.spawn().unwrap();
            let pid = i32::try_from(child.id()).unwrap();
            let me = std::env::current_exe().unwrap();
            while std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|e| e == me)
                || std::fs::read(format!("/proc/{pid}/environ")).is_ok_and(|e| e.is_empty())
            {
                std::thread::yield_now(); // `spawn` returns after the exec; this is a safety net
            }
            let exe = std::fs::read_link(format!("/proc/{pid}/exe")).unwrap();
            let exe: &'static str = Box::leak(exe.to_string_lossy().into_owned().into_boxed_str());
            let showers: &'static [&'static str] = Box::leak(vec![exe].into_boxed_slice());
            let peer = PeerIdentity::of_process(pid, showers);
            let bus = Bus::default();
            let (srv, mut cli) = UnixStream::pair().unwrap();
            let h = ask(&cli, b"subscribe account\n");
            serve(
                srv,
                Some(peer),
                &Empty,
                &bus,
                &AllowAll,
                &portal::State::default(),
            );
            h.join().unwrap();
            if subscribes {
                assert_eq!(bus.len(), 1, "the real shower subscribes");
            } else {
                let mut out = String::new();
                cli.read_to_string(&mut out).unwrap();
                assert!(out.contains("permission denied"), "{out}");
                assert_eq!(bus.len(), 0, "a preloaded shower is refused");
            }
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }

    /// Child half of `a_peer_that_execs_after_sending_is_refused`: runs only when the parent test
    /// re-executes this binary with `WOL_EXEC_CLIENT` set. Connects, sends a call, waits for the
    /// parent to accept, then exec()s into another binary before the server reads the request.
    #[test]
    fn exec_client_helper() {
        use std::os::unix::process::CommandExt;
        let Some(path) = std::env::var_os("WOL_EXEC_CLIENT") else {
            return;
        };
        let mut s = UnixStream::connect(path).unwrap();
        s.write_all(b"call {\"call\":\"notification.post\",\"title\":\"Hi\"}\n")
            .unwrap();
        if std::env::var_os("WOL_EXEC_EARLY").is_some() {
            // Before wold accepts: the request waits in the socket under the new image.
            let e = std::process::Command::new("/bin/sleep").arg("5").exec();
            panic!("exec: {e}");
        }
        let mut go = [0u8; 1];
        s.read_exact(&mut go).unwrap();
        let e = std::process::Command::new("/bin/sleep").arg("5").exec();
        panic!("exec: {e}");
    }

    #[test]
    fn a_peer_that_execs_after_sending_is_refused() {
        let dir = std::env::temp_dir().join(format!("wol-exec-{}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        let listener = UnixListener::bind(&dir).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "server::tests::exec_client_helper",
                "--nocapture",
            ])
            .env("WOL_EXEC_CLIENT", &dir)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let (mut conn, _) = listener.accept().unwrap();
        let peer = PeerIdentity::capture(&conn).unwrap();
        let me = std::env::current_exe().unwrap();
        assert_eq!(
            peer.exe(),
            Some(&*me.to_string_lossy()),
            "captured at accept"
        );
        conn.write_all(b"g").unwrap();
        let pid = peer.pid;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .as_deref()
            == Some(&*me)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the child never exec()ed"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            peer.caller().is_none(),
            "a changed executable is not trusted"
        );

        let bus = Bus::default();
        let shower = call("subscribe account\n", &bus);
        shower.set_nonblocking(true).unwrap();
        serve(
            conn,
            Some(peer),
            &Empty,
            &bus,
            &AllowAll,
            &portal::State::default(),
        );
        let mut line = String::new();
        let _ = BufReader::new(&shower).read_line(&mut line);
        assert_eq!(
            line, "",
            "no notification in the name of the exec()ed binary"
        );
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn an_overlong_request_is_dropped() {
        let bus = Bus::default();
        let (srv, mut cli) = UnixStream::pair().unwrap();
        let long = vec![b'x'; shared::portal::MAX_LINE + 10];
        std::thread::spawn(move || {
            shared::jobs::answer_hello(&cli).unwrap();
            let _ = cli.write_all(&long);
            let mut out = String::new();
            let _ = cli.read_to_string(&mut out);
            out
        });
        serve_me(srv, &Empty, &bus, &AllowAll, &portal::State::default());
        assert_eq!(bus.len(), 0);
    }
    /// Wait until `pid` no longer runs `exe`.
    fn wait_exec(pid: i32, exe: &std::path::Path) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .as_deref()
            == Some(exe)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the child never exec()ed"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// ADR 0033 regression: the client writes its request and exec()s before wold accepts, so
    /// the kernel names the new binary as the peer. The request must not be acted on.
    #[test]
    fn a_request_written_before_accept_by_a_process_that_execs_is_ignored() {
        let dir = std::env::temp_dir().join(format!("wol-exec-early-{}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        let listener = UnixListener::bind(&dir).unwrap();
        let me = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(&me)
            .args([
                "--exact",
                "server::tests::exec_client_helper",
                "--nocapture",
            ])
            .env("WOL_EXEC_CLIENT", &dir)
            .env("WOL_EXEC_EARLY", "1")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        wait_exec(i32::try_from(child.id()).unwrap(), &me);
        let (conn, _) = listener.accept().unwrap();
        let peer = PeerIdentity::capture(&conn).unwrap();
        assert!(
            peer.caller().is_some(),
            "the kernel vouches for the new image"
        );
        let bus = Bus::default();
        let shower = call("subscribe account\n", &bus);
        shower.set_nonblocking(true).unwrap();
        serve(
            conn,
            Some(peer),
            &Empty,
            &bus,
            &AllowAll,
            &portal::State::default(),
        );
        let mut line = String::new();
        let _ = BufReader::new(&shower).read_line(&mut line);
        assert_eq!(line, "", "the pre-written request never ran");
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn a_client_that_skips_the_hello_gets_no_answer() {
        let bus = Bus::default();
        let (srv, mut cli) = UnixStream::pair().unwrap();
        cli.write_all(b"list\n").unwrap();
        serve_me(srv, &Empty, &bus, &AllowAll, &portal::State::default());
        // The refusal comes at the first byte, so the rest of the line stays unread and the close
        // may reach the client as a reset: read what arrived before it.
        let mut got = Vec::new();
        let mut buf = [0u8; 256];
        while let Ok(n @ 1..) = cli.read(&mut buf) {
            got.extend_from_slice(&buf[..n]);
        }
        let out = String::from_utf8_lossy(&got);
        assert!(
            out.starts_with("hello ") && out.lines().count() == 1,
            "{out}"
        );
    }
}
