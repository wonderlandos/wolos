// SPDX-License-Identifier: BSD-2-Clause
//! The process's accessibility hub: what the native windows publish (their trees) and what an
//! assistive technology asked for (actions), shared between the engine's thread and the bus
//! threads of [`super::bridge`]. The engine never waits on the bus: it publishes a snapshot and
//! drains actions; the bus threads answer queries from the snapshots and wake the engine through
//! a socket pair (its descriptor joins the native windows' wait set).

use super::tree::{A11yNode, Tree};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// What an assistive technology asked a node to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// The default action (click, toggle).
    Default,
    Focus,
    /// Caret at a character offset (text fields).
    Caret(usize),
    /// Selection between character offsets (text fields).
    Select(usize, usize),
    /// A slider's value.
    Value(f64),
    ScrollTo,
}

/// One window's exported tree, with an index by node id.
pub struct Exported {
    pub slot: u32,
    pub tree: Tree,
    pub by_id: HashMap<u32, usize>,
}

impl Exported {
    pub fn node(&self, id: u32) -> Option<&A11yNode> {
        self.by_id.get(&id).and_then(|i| self.tree.nodes.get(*i))
    }
}

#[derive(Default)]
pub struct State {
    pub app_id: String,
    pub windows: Vec<Exported>,
    /// `(slot, node id, action)`.
    pub actions: Vec<(u32, u32, Action)>,
    /// The a11y bus connection while an assistive technology listens.
    pub conn: Option<zbus::blocking::Connection>,
    /// The registry's root, our root's parent (bus name, path).
    pub parent: Option<(String, String)>,
    /// `Application.Id` as the registry set it.
    pub app_number: i32,
    next_slot: u32,
}

/// Actions kept at most before older ones are dropped.
pub const MAX_ACTIONS: usize = 256;

pub struct Hub {
    state: Mutex<State>,
    active: AtomicBool,
    /// Bumped when export starts, so windows publish again from scratch.
    generation: AtomicU64,
    wake_tx: UnixStream,
    wake_rx: UnixStream,
}

static HUB: Mutex<Option<Arc<Hub>>> = Mutex::new(None);

/// The process's hub, made on first use; `start` runs once with the new hub (the status watch).
/// `None` only if the process cannot make a socket pair.
pub fn hub(app_id: &str, start: impl FnOnce(&Arc<Hub>)) -> Option<Arc<Hub>> {
    let mut g = HUB.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = g.as_ref() {
        return Some(h.clone());
    }
    let h = Arc::new(Hub::new(app_id)?);
    *g = Some(h.clone());
    drop(g);
    start(&h);
    Some(h)
}

impl Hub {
    /// A hub not shared with the process (tests).
    pub fn new(app_id: &str) -> Option<Hub> {
        let (rx, tx) = UnixStream::pair().ok()?;
        rx.set_nonblocking(true).ok()?;
        tx.set_nonblocking(true).ok()?;
        Some(Hub {
            state: Mutex::new(State {
                app_id: app_id.to_owned(),
                ..Default::default()
            }),
            active: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            wake_tx: tx,
            wake_rx: rx,
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether the trees are being exported (an assistive technology listens).
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Export starts (with this connection) or stops (`None`).
    pub fn set_connection(&self, conn: Option<zbus::blocking::Connection>) {
        let on = conn.is_some();
        {
            let mut s = self.lock();
            s.conn = conn;
            if !on {
                s.parent = None;
                for w in &mut s.windows {
                    w.tree = Tree::default();
                    w.by_id.clear();
                }
            }
        }
        self.active.store(on, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.wake();
    }

    /// A slot for a new window (its object paths use it).
    pub fn register(&self) -> u32 {
        let mut s = self.lock();
        s.next_slot += 1;
        let slot = s.next_slot;
        s.windows.push(Exported {
            slot,
            tree: Tree::default(),
            by_id: HashMap::new(),
        });
        slot
    }

    pub fn unregister(&self, slot: u32) {
        let mut s = self.lock();
        s.windows.retain(|w| w.slot != slot);
        s.actions.retain(|a| a.0 != slot);
    }

    /// Replace a window's tree; the old one is returned for the change signals.
    pub fn publish(&self, slot: u32, tree: Tree) -> Option<Tree> {
        let mut s = self.lock();
        let w = s.windows.iter_mut().find(|w| w.slot == slot)?;
        w.by_id = tree
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i))
            .collect();
        Some(std::mem::replace(&mut w.tree, tree))
    }

    /// Queue an action from the bus and wake the engine.
    pub fn push_action(&self, slot: u32, node: u32, a: Action) {
        {
            let mut s = self.lock();
            if s.actions.len() >= MAX_ACTIONS {
                s.actions.remove(0);
            }
            s.actions.push((slot, node, a));
        }
        self.wake();
    }

    /// The actions for one window, oldest first.
    pub fn take_actions(&self, slot: u32) -> Vec<(u32, Action)> {
        let mut s = self.lock();
        let mut out = Vec::new();
        s.actions.retain(|(sl, n, a)| {
            if *sl == slot {
                out.push((*n, a.clone()));
                false
            } else {
                true
            }
        });
        out
    }

    pub fn wake(&self) {
        // A full socket already means "awake".
        let _ = (&self.wake_tx).write(&[1]);
    }

    pub fn wake_fd(&self) -> RawFd {
        self.wake_rx.as_raw_fd()
    }

    pub fn drain_wake(&self) {
        let mut b = [0u8; 64];
        while matches!((&self.wake_rx).read(&mut b), Ok(n) if n > 0) {}
    }
}

/// The process's hub if one was made (a native window exists).
pub fn current() -> Option<Arc<Hub>> {
    HUB.lock().ok().and_then(|g| g.clone())
}
