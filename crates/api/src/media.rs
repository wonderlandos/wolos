// SPDX-License-Identifier: GPL-2.0-or-later
//! Players libwol apps publish (`wol_media_publish`). A player is a held connection like a search
//! provider: it exists while its process keeps the connection, one per process. wold only keeps
//! the list of who may be sent a remote control; what the player shows reaches the desktop as
//! events (`Event::MediaState`, `MediaGone`), and the desktop's keys come back as `Event::MediaKey`
//! to that one process, so no app can press another app's keys.
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Table {
    last: u32,
    players: Vec<Player>,
}

struct Player {
    token: u32,
    uid: u32,
    pid: i32,
}

#[derive(Default)]
pub struct Players(Arc<Mutex<Table>>);

/// Registered until dropped: the player's connection closed.
pub struct Registered {
    table: Arc<Mutex<Table>>,
    token: u32,
}

fn lock(t: &Mutex<Table>) -> std::sync::MutexGuard<'_, Table> {
    t.lock().unwrap_or_else(|p| p.into_inner())
}

// The player is no longer one: its connection closed or its guard was dropped.
impl Drop for Registered {
    fn drop(&mut self) {
        lock(&self.table).players.retain(|p| p.token != self.token);
    }
}

impl Players {
    /// `uid`'s process `pid` publishes a player. None when it already does: one per process.
    pub fn register(&self, uid: u32, pid: i32) -> Option<Registered> {
        let mut t = lock(&self.0);
        if t.players.iter().any(|p| p.uid == uid && p.pid == pid) {
            return None;
        }
        t.last = t.last.wrapping_add(1).max(1);
        let token = t.last;
        t.players.push(Player { token, uid, pid });
        Some(Registered {
            table: self.0.clone(),
            token,
        })
    }

    /// Whether process `pid` of account `uid` publishes a player now.
    pub fn is_player(&self, uid: u32, pid: i32) -> bool {
        lock(&self.0)
            .players
            .iter()
            .any(|p| p.uid == uid && p.pid == pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_player_per_process_and_it_ends_with_its_guard() {
        let p = Players::default();
        let a = p.register(1000, 7).expect("first");
        assert!(p.is_player(1000, 7));
        assert!(p.register(1000, 7).is_none(), "one per process");
        assert!(!p.is_player(1001, 7), "the account is part of the name");
        assert!(!p.is_player(1000, 8));
        let b = p.register(1000, 8).expect("another process");
        drop(a);
        assert!(!p.is_player(1000, 7));
        assert!(p.is_player(1000, 8));
        drop(b);
        assert!(p.register(1000, 7).is_some(), "free again");
    }
}
