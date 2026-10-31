// SPDX-License-Identifier: GPL-2.0-or-later
//! Apps as search providers for libwol (ADR 0020). A provider stays registered as long as its
//! connection to wold; a query goes only to the providers of that account, each as its own
//! process, and a reply counts only from a process that was asked.
use crate::bus::Bus;
use crate::portal::Caller;
use serde_json::{Value, json};
use shared::events::Event;
use shared::portal::SearchResult;
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Table {
    last: u32,
    providers: Vec<Provider>,
    pending: HashMap<u32, Pending>,
}

struct Provider {
    token: u32,
    uid: u32,
    pid: i32,
}

struct Pending {
    uid: u32,
    waiting: Vec<i32>,
    rows: Vec<Value>,
}

#[derive(Default)]
struct Shared {
    table: Mutex<Table>,
    replied: Condvar,
}

#[derive(Default)]
pub struct Searches(Arc<Shared>);

/// Registered until dropped: the provider's connection closed.
pub struct Registered {
    shared: Arc<Shared>,
    token: u32,
}

impl Drop for Registered {
    fn drop(&mut self) {
        lock(&self.shared)
            .providers
            .retain(|p| p.token != self.token);
    }
}

fn lock(s: &Shared) -> std::sync::MutexGuard<'_, Table> {
    s.table.lock().unwrap_or_else(|p| p.into_inner())
}

impl Searches {
    pub fn register(&self, c: &Caller) -> Registered {
        let mut t = lock(&self.0);
        t.last = t.last.wrapping_add(1).max(1);
        let token = t.last;
        t.providers.push(Provider {
            token,
            uid: c.uid,
            pid: c.pid,
        });
        Registered {
            shared: self.0.clone(),
            token,
        }
    }

    /// Ask the providers of `uid`; what they answered within `wait`.
    pub fn collect<W: Write + Send>(
        &self,
        bus: &Bus<W>,
        uid: u32,
        text: &str,
        wait: Duration,
    ) -> Vec<Value> {
        let (id, pids) = {
            let mut t = lock(&self.0);
            let pids: Vec<i32> = t
                .providers
                .iter()
                .filter(|p| p.uid == uid)
                .map(|p| p.pid)
                .collect();
            t.last = t.last.wrapping_add(1).max(1);
            let id = t.last;
            t.pending.insert(
                id,
                Pending {
                    uid,
                    waiting: pids.clone(),
                    rows: Vec::new(),
                },
            );
            (id, pids)
        };
        for pid in pids {
            bus.publish(&Event::SearchQuery {
                uid,
                pid,
                id,
                text: text.to_owned(),
            });
        }
        let end = Instant::now() + wait;
        let mut t = lock(&self.0);
        loop {
            let done = t.pending.get(&id).is_none_or(|p| p.waiting.is_empty());
            let left = end.saturating_duration_since(Instant::now());
            if done || left.is_zero() {
                return t.pending.remove(&id).map_or_else(Vec::new, |p| p.rows);
            }
            t = self
                .0
                .replied
                .wait_timeout(t, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    /// A provider answers query `id`. False unless that process was asked and has not answered.
    pub fn reply(&self, c: &Caller, id: u32, results: Vec<SearchResult>) -> bool {
        let mut t = lock(&self.0);
        let Some(p) = t.pending.get_mut(&id).filter(|p| p.uid == c.uid) else {
            return false;
        };
        let Some(i) = p.waiting.iter().position(|w| *w == c.pid) else {
            return false;
        };
        p.waiting.remove(i);
        p.rows.extend(results.into_iter().map(|r| {
            json!({
                "key": r.key, "title": r.title, "detail": r.detail,
                "app": c.app, "provider": c.pid,
            })
        }));
        self.0.replied.notify_all();
        true
    }

    /// Is `pid` a provider of account `uid`?
    pub fn is_provider(&self, uid: u32, pid: i32) -> bool {
        lock(&self.0)
            .providers
            .iter()
            .any(|p| p.uid == uid && p.pid == pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    fn who(uid: u32, pid: i32) -> Caller {
        Caller {
            uid,
            pid,
            app: "/usr/bin/notes".into(),
        }
    }

    fn row(key: &str) -> SearchResult {
        SearchResult {
            key: key.into(),
            title: "A note".into(),
            detail: String::new(),
        }
    }

    #[test]
    fn a_query_reaches_only_that_accounts_providers_and_only_they_reply() {
        let s = Arc::new(Searches::default());
        let bus = Arc::new(Bus::default());
        let mine = s.register(&who(1000, 10));
        let _other = s.register(&who(1001, 20));
        let (app, seen) = UnixStream::pair().unwrap();
        bus.subscribe(None, Some((1000, 10)), app);
        let (app2, seen2) = UnixStream::pair().unwrap();
        bus.subscribe(None, Some((1001, 20)), app2);

        let (s2, bus2) = (s.clone(), bus.clone());
        let t =
            std::thread::spawn(move || s2.collect(&bus2, 1000, "note", Duration::from_secs(10)));
        let mut line = String::new();
        BufReader::new(&seen).read_line(&mut line).unwrap();
        let Some(Event::SearchQuery { id, text, .. }) = Event::parse(&line) else {
            panic!("{line}")
        };
        assert_eq!(text, "note");
        assert!(!s.reply(&who(1001, 20), id, vec![row("x")]), "not asked");
        assert!(!s.reply(&who(1000, 11), id, vec![row("x")]), "not asked");
        assert!(s.reply(&who(1000, 10), id, vec![row("a")]));
        assert!(!s.reply(&who(1000, 10), id, vec![row("b")]), "answers once");
        let rows = t.join().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["key"], "a");
        assert_eq!(rows[0]["provider"], 10);
        seen2.set_nonblocking(true).unwrap();
        let mut none = String::new();
        let _ = BufReader::new(&seen2).read_line(&mut none);
        assert_eq!(none, "", "another account's provider saw nothing");

        assert!(s.is_provider(1000, 10));
        drop(mine);
        assert!(!s.is_provider(1000, 10), "gone with its connection");
    }

    #[test]
    fn a_silent_provider_costs_only_the_wait() {
        let s = Searches::default();
        let bus = Bus::<UnixStream>::default();
        let _p = s.register(&who(1000, 10));
        let rows = s.collect(&bus, 1000, "q", Duration::from_millis(20));
        assert!(rows.is_empty());
        assert!(lock(&s.0).pending.is_empty());
    }
}
