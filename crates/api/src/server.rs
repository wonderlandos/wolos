//! Socket glue for wold: one request line per connection; `subscribe` keeps the connection.
use crate::bus::Bus;
use crate::control::{JobTable, handle};
use crate::packages;
use crate::portal::{self, Caller, Grants};
use shared::events::Event;
use shared::jobs::{JobRequest, Reply};
use shared::sys::{FdReader, send_with_fd};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

/// The calling app: peer uid and the canonical executable of the peer pid (kernel facts).
pub fn caller(peer: Option<shared::sys::Peer>) -> Option<Caller> {
    let p = peer?;
    let mut buf = [0u8; 4096];
    let n = shared::sys::executable(p.pid, &mut buf)?;
    let app = std::str::from_utf8(&buf[..n]).ok()?.to_owned();
    app.starts_with('/').then_some(Caller {
        uid: p.uid,
        pid: p.pid,
        app,
    })
}

/// Serve one control connection. `peer` comes from peer credentials.
pub fn serve(
    stream: UnixStream,
    peer: Option<shared::sys::Peer>,
    jobs: &dyn JobTable,
    bus: &Bus<UnixStream>,
    grants: &dyn Grants,
    state: &portal::State,
) {
    let uid = peer.map(|p| p.uid);
    let who = peer.map(|p| (p.uid, p.pid));
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let mut line = String::new();
    let mut reader = BufReader::new(FdReader::new(stream.try_clone().expect("clone socket")));
    if reader
        .by_ref()
        .take(64 * 1024)
        .read_line(&mut line)
        .is_err()
    {
        return;
    }
    let attached = reader.get_mut().fd.take();
    let req = JobRequest::parse(&line);
    match &req {
        Some(JobRequest::Subscribe) => {
            let _ = stream.set_read_timeout(None);
            bus.subscribe(None, who, stream);
        }
        Some(JobRequest::SubscribeAccount) => match (uid, caller(peer)) {
            (Some(uid), Some(c)) if grants.shows_notifications(&c.app) => {
                let _ = stream.set_read_timeout(None);
                bus.subscribe(Some(uid), who, stream);
            }
            _ => {
                let mut w = &stream;
                let _ = writeln!(
                    w,
                    "{}",
                    Reply::of(Some("permission denied: notification shower only".into()))
                );
            }
        },
        Some(JobRequest::Publish(json)) => {
            if let Some(e) = Event::parse(json).filter(Event::client_may_publish) {
                bus.publish(&e);
            }
        }
        Some(JobRequest::Call(json)) => {
            let who = caller(peer);
            let answer = if packages::is_package_call(json) {
                // ADR 0021: wopkg's system changes; the same consent grants, the real machine.
                let c = who.as_ref().map(|c| (c.uid, c.app.as_str()));
                packages::handle(
                    json,
                    c,
                    &|app, cap| grants.granted(app, cap),
                    &packages::Host,
                )
            } else {
                let (answer, extra) =
                    portal::respond(json, who.as_ref(), grants, bus, state, attached);
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
        cli.write_all(
            b"call {\"call\":\"package.change\",\"verb\":\"install\",\"names\":[\"vim\"]}\n",
        )
        .unwrap();
        serve(srv, me(), &Empty, &bus, &Deny, &portal::State::default());
        let mut out = String::new();
        cli.read_to_string(&mut out).unwrap();
        assert!(out.contains("not allowed to install system apps"), "{out}");
    }

    fn me() -> Option<shared::sys::Peer> {
        Some(shared::sys::Peer {
            pid: std::process::id() as i32,
            uid: 1000,
        })
    }

    fn call(line: &str, bus: &Bus<UnixStream>) -> UnixStream {
        let (srv, mut cli) = UnixStream::pair().unwrap();
        cli.write_all(line.as_bytes()).unwrap();
        serve(srv, me(), &Empty, bus, &AllowAll, &portal::State::default());
        cli
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
        assert_eq!(uid, 1000);
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
            app.write_all(b"call {\"call\":\"panel.open\",\"title\":\"Open\"}\n")
                .unwrap();
            t.spawn(|| serve(srv, me(), &Empty, &bus, &AllowAll, &state));

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
            send_with_fd(&desktop, reply.as_bytes(), file.as_fd()).unwrap();
            serve(srv, me(), &Empty, &bus, &AllowAll, &state);
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
    fn only_the_shower_subscribes_to_account_events() {
        let bus = Bus::default();
        let (srv, mut cli) = UnixStream::pair().unwrap();
        cli.write_all(b"subscribe account\n").unwrap();
        serve(srv, me(), &Empty, &bus, &Strict, &portal::State::default());
        let mut out = String::new();
        cli.read_to_string(&mut out).unwrap();
        assert!(out.contains("permission denied"), "{out}");
        assert_eq!(bus.len(), 0);
    }
}
