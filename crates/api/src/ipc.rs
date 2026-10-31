// SPDX-License-Identifier: GPL-2.0-or-later
//! Messages between apps of one account (ADR 0020). An app is addressed by its executable, which
//! the kernel vouches for, so no app can take over another's name; the receiver sees the sender
//! the same way. An endpoint lasts as long as its connection to wold.
use crate::bus::Bus;
use crate::portal::Caller;
use shared::events::Event;
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Messages one account can have waiting for a reply.
pub const MAX_WAITING: usize = 64;

#[derive(Default)]
struct Table {
    last: u32,
    endpoints: Vec<Endpoint>,
    waiting: HashMap<u32, Waiting>,
}

struct Endpoint {
    token: u32,
    uid: u32,
    pid: i32,
    app: String,
    name: String,
}

struct Waiting {
    uid: u32,
    to: i32,
    reply: Option<String>,
}

#[derive(Default)]
struct Shared {
    table: Mutex<Table>,
    replied: Condvar,
}

#[derive(Default)]
pub struct Endpoints(Arc<Shared>);

/// Listening until dropped: the connection closed.
pub struct Listening {
    shared: Arc<Shared>,
    token: u32,
}

impl Drop for Listening {
    fn drop(&mut self) {
        lock(&self.shared)
            .endpoints
            .retain(|e| e.token != self.token);
    }
}

fn lock(s: &Shared) -> std::sync::MutexGuard<'_, Table> {
    s.table.lock().unwrap_or_else(|p| p.into_inner())
}

fn next(t: &mut Table) -> u32 {
    t.last = t.last.wrapping_add(1).max(1);
    t.last
}

impl Endpoints {
    /// None when a process of this app already listens under `name`.
    pub fn listen(&self, c: &Caller, name: &str) -> Option<Listening> {
        let mut t = lock(&self.0);
        let taken = |e: &Endpoint| e.uid == c.uid && e.app == c.app && e.name == name;
        if t.endpoints.iter().any(taken) {
            return None;
        }
        let token = next(&mut t);
        t.endpoints.push(Endpoint {
            token,
            uid: c.uid,
            pid: c.pid,
            app: c.app.clone(),
            name: name.to_owned(),
        });
        Some(Listening {
            shared: self.0.clone(),
            token,
        })
    }

    /// Hand `body` to `peer`'s endpoint; its reply, or a sentence for the sender.
    pub fn send<W: Write + Send>(
        &self,
        bus: &Bus<W>,
        c: &Caller,
        (peer, name): (&str, &str),
        body: String,
        wait: Duration,
    ) -> Result<String, &'static str> {
        let (id, pid) = {
            let mut t = lock(&self.0);
            let found = t
                .endpoints
                .iter()
                .find(|e| e.uid == c.uid && e.app == peer && e.name == name);
            let Some(pid) = found.map(|e| e.pid) else {
                return Err("That app is not listening.");
            };
            if t.waiting.values().filter(|w| w.uid == c.uid).count() >= MAX_WAITING {
                return Err("Too many messages are waiting for a reply.");
            }
            let id = next(&mut t);
            let w = Waiting {
                uid: c.uid,
                to: pid,
                reply: None,
            };
            t.waiting.insert(id, w);
            (id, pid)
        };
        bus.publish(&Event::Message {
            uid: c.uid,
            pid,
            id,
            from: c.app.clone(),
            name: name.to_owned(),
            body,
        });
        let end = Instant::now() + wait;
        let mut t = lock(&self.0);
        loop {
            let left = end.saturating_duration_since(Instant::now());
            let got = t.waiting.get(&id).is_none_or(|w| w.reply.is_some());
            if got || left.is_zero() {
                let reply = t.waiting.remove(&id).and_then(|w| w.reply);
                return reply.ok_or("That app did not reply.");
            }
            t = self
                .0
                .replied
                .wait_timeout(t, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    /// The process a message went to answers it, once. False otherwise.
    pub fn reply(&self, c: &Caller, id: u32, body: String) -> bool {
        let mut t = lock(&self.0);
        let Some(w) = t.waiting.get_mut(&id) else {
            return false;
        };
        if w.uid != c.uid || w.to != c.pid || w.reply.is_some() {
            return false;
        }
        w.reply = Some(body);
        self.0.replied.notify_all();
        true
    }

    /// Hand `body` to every endpoint named `topic` whose app allowed hearing this one.
    pub fn publish<W: Write + Send>(
        &self,
        bus: &Bus<W>,
        c: &Caller,
        topic: &str,
        body: &str,
        hears: impl Fn(&str) -> bool,
    ) {
        let to: Vec<i32> = lock(&self.0)
            .endpoints
            .iter()
            .filter(|e| e.uid == c.uid && e.name == topic && hears(&e.app))
            .map(|e| e.pid)
            .collect();
        for pid in to {
            bus.publish(&Event::Message {
                uid: c.uid,
                pid,
                id: 0,
                from: c.app.clone(),
                name: topic.to_owned(),
                body: body.to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    fn app(uid: u32, pid: i32, exe: &str) -> Caller {
        Caller {
            uid,
            pid,
            app: exe.into(),
        }
    }

    const NOTES: &str = "/usr/bin/notes";
    const MAIL: &str = "/usr/bin/mail";

    fn inbox(bus: &Bus<UnixStream>, uid: u32, pid: i32) -> UnixStream {
        let (w, r) = UnixStream::pair().unwrap();
        bus.subscribe(None, Some((uid, pid)), w);
        r
    }

    fn message(r: &UnixStream) -> Event {
        let mut line = String::new();
        BufReader::new(r).read_line(&mut line).unwrap();
        Event::parse(&line).unwrap()
    }

    #[test]
    fn a_message_names_its_sender_and_only_the_receiver_replies() {
        let e = Arc::new(Endpoints::default());
        let bus = Arc::new(Bus::default());
        let notes = app(1000, 10, NOTES);
        let listening = e.listen(&notes, "greet").unwrap();
        assert!(e.listen(&app(1000, 11, NOTES), "greet").is_none(), "taken");
        let got = inbox(&bus, 1000, 10);

        let (e2, bus2) = (e.clone(), bus.clone());
        let t = std::thread::spawn(move || {
            let to = (NOTES, "greet");
            let wait = Duration::from_secs(10);
            e2.send(&bus2, &app(1000, 20, MAIL), to, "hi".into(), wait)
        });
        let Event::Message { id, from, body, .. } = message(&got) else {
            panic!()
        };
        assert_eq!((from.as_str(), body.as_str()), (MAIL, "hi"));
        assert!(!e.reply(&app(1000, 20, MAIL), id, "x".into()), "not its");
        assert!(!e.reply(&app(1001, 10, NOTES), id, "x".into()), "account");
        assert!(e.reply(&notes, id, "hello".into()));
        assert!(!e.reply(&notes, id, "twice".into()));
        assert_eq!(t.join().unwrap(), Ok("hello".into()));
        assert!(!e.reply(&notes, id, "late".into()));

        drop(listening);
        let w = Duration::ZERO;
        let to = (NOTES, "greet");
        let err = e.send(&bus, &app(1000, 20, MAIL), to, "hi".into(), w);
        assert_eq!(err, Err("That app is not listening."));
    }

    #[test]
    fn another_account_has_no_such_endpoint() {
        let e = Endpoints::default();
        let bus = Bus::<UnixStream>::default();
        let _l = e.listen(&app(1000, 10, NOTES), "greet").unwrap();
        let s = app(1001, 20, MAIL);
        let got = e.send(&bus, &s, (NOTES, "greet"), "x".into(), Duration::ZERO);
        assert_eq!(got, Err("That app is not listening."));
    }

    #[test]
    fn a_receiver_that_stays_silent_costs_only_the_wait() {
        let e = Endpoints::default();
        let bus = Bus::<UnixStream>::default();
        let _l = e.listen(&app(1000, 10, NOTES), "greet").unwrap();
        let s = app(1000, 20, MAIL);
        let w = Duration::from_millis(10);
        let got = e.send(&bus, &s, (NOTES, "greet"), "x".into(), w);
        assert_eq!(got, Err("That app did not reply."));
        assert!(lock(&e.0).waiting.is_empty());
    }

    #[test]
    fn waiting_messages_are_bounded() {
        let e = Endpoints::default();
        let bus = Bus::<UnixStream>::default();
        let _l = e.listen(&app(1000, 10, NOTES), "greet").unwrap();
        for id in 1..=MAX_WAITING as u32 {
            let w = Waiting {
                uid: 1000,
                to: 10,
                reply: None,
            };
            lock(&e.0).waiting.insert(id + 100, w);
        }
        let s = app(1000, 20, MAIL);
        let got = e.send(&bus, &s, (NOTES, "greet"), "x".into(), Duration::ZERO);
        assert_eq!(got, Err("Too many messages are waiting for a reply."));
    }

    #[test]
    fn a_topic_reaches_only_apps_that_allowed_the_publisher() {
        let e = Endpoints::default();
        let bus = Bus::<UnixStream>::default();
        let _a = e.listen(&app(1000, 10, NOTES), "news").unwrap();
        let _b = e.listen(&app(1000, 11, MAIL), "news").unwrap();
        let _c = e.listen(&app(1001, 12, NOTES), "news").unwrap();
        let (yes, no, other) = (
            inbox(&bus, 1000, 10),
            inbox(&bus, 1000, 11),
            inbox(&bus, 1001, 12),
        );
        let p = app(1000, 20, "/usr/bin/feed");
        e.publish(&bus, &p, "news", "item", |a| a == NOTES);
        let Event::Message { id, .. } = message(&yes) else {
            panic!()
        };
        assert_eq!(id, 0, "nothing to reply to");
        for quiet in [no, other] {
            quiet.set_nonblocking(true).unwrap();
            let mut s = String::new();
            let _ = BufReader::new(&quiet).read_line(&mut s);
            assert_eq!(s, "");
        }
    }
}
