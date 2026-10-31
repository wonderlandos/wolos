//! Event bus: `subscribe` connections receive every published `shared::events::Event` line, except
//! events for one account or process (`Event::audience`): account events reach only subscribers
//! wold admitted for that account (`subscribe account`, the notification shower); process events
//! only that process's own subscription.
use shared::events::{Audience, Event};
use std::io::Write;
use std::sync::Mutex;

pub struct Bus<W: Write + Send> {
    subscribers: Mutex<Vec<Sub<W>>>,
}

struct Sub<W> {
    /// Admitted for this account's events.
    account: Option<u32>,
    /// (uid, pid) from peer credentials.
    peer: Option<(u32, i32)>,
    w: W,
}

impl<W: Write + Send> Default for Bus<W> {
    fn default() -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
        }
    }
}

impl<W: Write + Send> Bus<W> {
    /// `account`: also receive this account's events (None = public events only). `peer`:
    /// (uid, pid) of the subscriber, for events meant for that process.
    pub fn subscribe(&self, account: Option<u32>, peer: Option<(u32, i32)>, w: W) {
        self.lock().push(Sub { account, peer, w });
    }

    /// Fan out to every subscriber in the event's audience; dead ones are dropped.
    /// Returns how many received it.
    pub fn publish(&self, e: &Event) -> usize {
        let line = e.to_line();
        let audience = e.audience();
        let mut sent = 0;
        self.lock().retain_mut(|sub| {
            let meant = match audience {
                Audience::Everyone => true,
                Audience::Account(u) => sub.account == Some(u),
                Audience::Process { uid, pid } => sub.peer == Some((uid, pid)),
            };
            if !meant {
                return true;
            }
            let w = &mut sub.w;
            let ok = w
                .write_all(line.as_bytes())
                .and_then(|()| w.flush())
                .is_ok();
            sent += usize::from(ok);
            ok
        });
        sent
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Sub<W>>> {
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
}
