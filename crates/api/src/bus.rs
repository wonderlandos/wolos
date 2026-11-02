// SPDX-License-Identifier: GPL-2.0-or-later
//! Event bus: `subscribe` connections receive every published `shared::events::Event` line, except
//! events for one account or process (`Event::audience`): account events reach only subscribers
//! wold admitted for that account (`subscribe account`, the notification shower); process events
//! only that process's own subscription. Everyone events a client publishes reach only subscribers
//! of the publisher's own uid (and root); wold's own reach all (audit A11).
//! Invariant (audit B4/A6): no I/O while the subscriber list is locked. `publish` snapshots the
//! list, writes outside the lock, and drops a subscriber whose write fails; wold hands the bus
//! non-blocking sockets, so a subscriber that stops reading fills its buffer and is dropped
//! instead of stalling every publisher.
use shared::events::{Audience, Event};
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Subscriptions one uid may hold at once.
pub const MAX_PER_UID: usize = 64;
/// Subscriptions in total.
pub const MAX_TOTAL: usize = 1024;

pub struct Bus<W: Write + Send> {
    subscribers: Mutex<(u64, Vec<Arc<Sub<W>>>)>,
}

struct Sub<W> {
    id: u64,
    /// Admitted for this account's events.
    account: Option<u32>,
    /// (uid, pid) from peer credentials.
    peer: Option<(u32, i32)>,
    /// Locked only for one write, never together with the list.
    w: Mutex<W>,
}

impl<W> Sub<W> {
    fn uid(&self) -> Option<u32> {
        self.peer.map(|p| p.0).or(self.account)
    }

    fn meant(&self, audience: Audience, from: Option<u32>) -> bool {
        match audience {
            Audience::Everyone => match from {
                None | Some(0) => true,
                Some(u) => matches!(self.uid(), Some(s) if s == u || s == 0),
            },
            Audience::Account(u) => self.account == Some(u),
            Audience::Process { uid, pid } => self.peer == Some((uid, pid)),
        }
    }
}

impl<W: Write + Send> Default for Bus<W> {
    fn default() -> Self {
        Self {
            subscribers: Mutex::new((0, Vec::new())),
        }
    }
}

impl<W: Write + Send> Bus<W> {
    /// `account`: also receive this account's events (None = public events only). `peer`:
    /// (uid, pid) of the subscriber, for events meant for that process. False (and `w` dropped)
    /// when the uid or the bus is at its cap ([`MAX_PER_UID`], [`MAX_TOTAL`]).
    pub fn subscribe(&self, account: Option<u32>, peer: Option<(u32, i32)>, w: W) -> bool {
        let mut list = self.lock();
        let uid = peer.map(|p| p.0).or(account);
        let mine = list.1.iter().filter(|s| s.uid() == uid).count();
        if list.1.len() >= MAX_TOTAL || mine >= MAX_PER_UID {
            return false;
        }
        list.0 += 1;
        let id = list.0;
        list.1.push(Arc::new(Sub {
            id,
            account,
            peer,
            w: Mutex::new(w),
        }));
        true
    }

    /// An event wold itself makes: see [`Self::publish_from`].
    pub fn publish(&self, e: &Event) -> usize {
        self.publish_from(None, e)
    }

    /// Fan out to every subscriber in the event's audience; dead or slow ones are dropped.
    /// `from`: uid of the client that published it (None: wold). Returns how many received it.
    pub fn publish_from(&self, from: Option<u32>, e: &Event) -> usize {
        let line = e.to_line();
        let audience = e.audience();
        let targets: Vec<Arc<Sub<W>>> = self
            .lock()
            .1
            .iter()
            .filter(|s| s.meant(audience, from))
            .cloned()
            .collect();
        let mut dead = Vec::new();
        let mut sent = 0;
        for sub in targets {
            let mut w = sub.w.lock().unwrap_or_else(|p| p.into_inner());
            let ok = w
                .write_all(line.as_bytes())
                .and_then(|()| w.flush())
                .is_ok();
            sent += usize::from(ok);
            if !ok {
                dead.push(sub.id);
            }
        }
        if !dead.is_empty() {
            self.lock().1.retain(|s| !dead.contains(&s.id));
        }
        sent
    }

    pub fn len(&self) -> usize {
        self.lock().1.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (u64, Vec<Arc<Sub<W>>>)> {
        self.subscribers.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    #[test]
    fn fans_out_and_drops_dead_subscribers() {
        let bus = Bus::default();
        let (a, a_peer) = UnixStream::pair().unwrap();
        let (b, b_peer) = UnixStream::pair().unwrap();
        bus.subscribe(None, None, a);
        bus.subscribe(Some(1000), None, b);
        drop(b_peer);
        let e = Event::Workspace { name: "2".into() };
        assert_eq!(bus.publish(&e), 1);
        let mut line = String::new();
        BufReader::new(a_peer).read_line(&mut line).unwrap();
        assert_eq!(Event::parse(&line), Some(e));
    }

    #[test]
    fn account_events_reach_only_their_account() {
        let bus = Bus::default();
        let mut peers = Vec::new();
        for uid in [Some(1000), Some(1001), Some(0), None] {
            let (s, p) = UnixStream::pair().unwrap();
            p.set_nonblocking(true).unwrap();
            bus.subscribe(uid, uid.map(|u| (u, 7)), s);
            peers.push(p);
        }
        let n = Event::Notification {
            uid: 1000,
            app: "/usr/bin/foo".into(),
            title: "t".into(),
            body: String::new(),
            id: 1,
            actions: Vec::new(),
            replace: false,
        };
        assert_eq!(bus.publish(&n), 1);
        let got: Vec<bool> = peers
            .iter()
            .map(|p| {
                let mut line = String::new();
                BufReader::new(p).read_line(&mut line).is_ok_and(|n| n > 0)
            })
            .collect();
        assert_eq!(got, [true, false, false, false]);
        assert_eq!(bus.len(), 4, "not meant is not dead");
        let to = |uid, pid| Event::NotificationAction {
            uid,
            pid,
            id: 1,
            key: "ok".into(),
        };
        assert_eq!(bus.publish(&to(1001, 7)), 1, "only (1001, 7)");
        assert_eq!(bus.publish(&to(1001, 8)), 0);
    }

    #[test]
    fn a_slow_subscriber_is_dropped_and_does_not_block_others() {
        let bus = Bus::default();
        let (slow, _slow_peer) = UnixStream::pair().unwrap(); // never read
        slow.set_nonblocking(true).unwrap();
        let (fast, fast_peer) = UnixStream::pair().unwrap();
        fast.set_nonblocking(true).unwrap();
        bus.subscribe(None, None, slow);
        bus.subscribe(None, None, fast);
        let mut r = BufReader::new(fast_peer);
        let e = Event::Workspace {
            name: "x".repeat(1000),
        };
        let started = std::time::Instant::now();
        let mut rounds = 0;
        while bus.len() == 2 && rounds < 100_000 {
            assert!(bus.publish(&e) >= 1, "the fast one always receives");
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            assert_eq!(Event::parse(&line).as_ref(), Some(&e));
            rounds += 1;
        }
        assert_eq!(bus.len(), 1, "the slow subscriber was dropped");
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(bus.publish(&e), 1);
    }

    #[test]
    fn client_events_reach_only_the_publishers_uid_and_root() {
        let bus = Bus::default();
        let mut peers = Vec::new();
        for uid in [1000, 1001, 0] {
            let (s, p) = UnixStream::pair().unwrap();
            p.set_nonblocking(true).unwrap();
            bus.subscribe(None, Some((uid, 7)), s);
            peers.push(p);
        }
        let e = Event::Workspace { name: "2".into() };
        assert_eq!(bus.publish_from(Some(1000), &e), 2, "1000 and root");
        assert_eq!(bus.publish_from(Some(0), &e), 3);
        assert_eq!(bus.publish(&e), 3, "wold's own reach everyone");
    }

    #[test]
    fn subscriptions_are_capped_per_uid() {
        let bus = Bus::default();
        let mut keep = Vec::new();
        for _ in 0..MAX_PER_UID {
            let (s, p) = UnixStream::pair().unwrap();
            assert!(bus.subscribe(None, Some((1000, 1)), s));
            keep.push(p);
        }
        let (s, _p) = UnixStream::pair().unwrap();
        assert!(!bus.subscribe(None, Some((1000, 1)), s));
        let (s, _q) = UnixStream::pair().unwrap();
        assert!(
            bus.subscribe(None, Some((1001, 1)), s),
            "another uid still may"
        );
    }
}
