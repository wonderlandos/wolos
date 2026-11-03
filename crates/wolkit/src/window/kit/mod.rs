// SPDX-License-Identifier: BSD-2-Clause
//! The kit backend: a window whose scene is lowered to a `shared::view` tree and shown by the Kit
//! host (the shell). The host keeps the window, draws it with Kit parts and sends clicks and edits
//! back; this module turns them into events. One bounded socket per engine, one named reader
//! thread, lines at most [`MAX_LINE`] bytes (the host's own limit).
//!
//! The wire is `ToHost`/`FromHost` of `shared::view`, unchanged: no GPL change is needed.

pub mod lower;

use super::route::receiver;
use super::{BACKEND_KIT, Backend, Sink, WindowCore, pipeline};
use crate::copy;
use crate::error::{Error, Result};
use crate::event::kind as ev;
use crate::loop_::Waker;
use crate::scene::SceneRead;
use crate::scene::ids::{Ix, to_u32};
use shared::bounded::BoundedLineReader;
use shared::view::{FromHost, MAX_TITLE, ToHost};
use std::collections::VecDeque;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Longest line from the host (`crates/shell/src/app_host.rs` MAX_LINE).
pub const MAX_LINE: usize = 256 * 1024;
/// Events from the host held before the reader waits for the engine to catch up.
const INBOX_MAX: usize = 1024;
pub const KIT_ENV: &str = "WOL_KIT_SOCKET";
pub const KIT_SOCKET: &str = "wol-kit.sock";

/// The Kit socket path: `$WOL_KIT_SOCKET`, else `$XDG_RUNTIME_DIR/wol-kit.sock`. The environment
/// comes in as a lookup (RR-027); the C boundary passes the process's.
pub fn socket_path(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    var(KIT_ENV)
        .map(PathBuf::from)
        .or_else(|| var("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join(KIT_SOCKET)))
}

struct Inbox {
    q: Mutex<VecDeque<FromHost>>,
    lost: AtomicBool,
}

/// The engine's connection to the Kit host, shared by its kit windows.
pub struct Conn {
    stream: Mutex<UnixStream>,
    inbox: Arc<Inbox>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Conn {
    /// Connect and say hello. `wake` is poked whenever the host sent something.
    pub fn connect(path: &Path, app_id: &str, wake: UnixStream) -> Result<Conn> {
        let gone = || Error::unavailable(copy::KIT_GONE);
        let stream = UnixStream::connect(path).map_err(|_| gone())?;
        shared::bounded::limit_socket(&stream, Duration::from_secs(10)).map_err(|_| gone())?;
        // The reader waits for the host as long as it takes; only writes have a deadline.
        stream.set_read_timeout(None).map_err(|_| gone())?;
        let hello = ToHost::Hello {
            app: app_id.to_owned(),
        };
        (&stream)
            .write_all(hello.to_line().as_bytes())
            .map_err(|_| gone())?;
        let inbox = Arc::new(Inbox {
            q: Mutex::default(),
            lost: AtomicBool::new(false),
        });
        let rx = stream.try_clone().map_err(|_| gone())?;
        let ib = inbox.clone();
        std::thread::Builder::new()
            .name("wolkit-kit-rx".into())
            .spawn(move || {
                let mut r = BoundedLineReader::new(rx, MAX_LINE, u64::MAX);
                while let Ok(Some(line)) = r.read_line_str() {
                    if let Some(e) = FromHost::parse(&line) {
                        let mut q = lock(&ib.q);
                        if q.len() < INBOX_MAX {
                            q.push_back(e);
                        }
                        drop(q);
                        let _ = (&wake).write(&[1]);
                    }
                }
                ib.lost.store(true, Ordering::Release);
                let _ = (&wake).write(&[1]);
            })
            .map_err(|_| gone())?;
        Ok(Conn {
            stream: Mutex::new(stream),
            inbox,
        })
    }

    fn send(&self, line: &str) -> Result<()> {
        if self.inbox.lost.load(Ordering::Acquire) {
            return Err(Error::unavailable(copy::KIT_LOST));
        }
        lock(&self.stream)
            .write_all(line.as_bytes())
            .map_err(|_| Error::unavailable(copy::KIT_LOST))
    }
}

// Shuts the socket down so the reader thread's read returns and the thread ends.
impl Drop for Conn {
    fn drop(&mut self) {
        let _ = lock(&self.stream).shutdown(std::net::Shutdown::Both);
    }
}

/// Make the engine's connection when the first kit window needs it.
pub fn connect_for(path: Option<&Path>, app_id: &str, waker: &Waker) -> Result<Arc<Conn>> {
    let path = path.ok_or_else(|| Error::unavailable(copy::KIT_GONE))?;
    let wake = waker
        .sender()
        .map_err(|_| Error::unavailable(copy::KIT_GONE))?;
    Ok(Arc::new(Conn::connect(path, app_id, wake)?))
}

pub struct KitWindow {
    conn: Arc<Conn>,
    /// The last line sent, so an unchanged tree sends nothing.
    last: String,
}

impl KitWindow {
    pub fn new(conn: Arc<Conn>) -> Self {
        KitWindow {
            conn,
            last: String::new(),
        }
    }

    /// Send the window's content (lowered by the caller) unless it is what the host has.
    fn present(&mut self, w: &mut WindowCore, content: shared::view::Node) -> Result<()> {
        let title: String = w.title.chars().filter(|c| !c.is_control()).collect();
        if title.len() > MAX_TITLE {
            return Err(Error::invalid("A window title is at most 256 bytes."));
        }
        let msg = ToHost::Show {
            window: w.id,
            title,
            content,
            menus: Vec::new(),
            toolbar: None,
        };
        let line = msg.to_line();
        if line == self.last {
            return Ok(());
        }
        self.conn.send(&line)?;
        self.last = line;
        Ok(())
    }
}

impl Backend for KitWindow {
    fn kind(&self) -> u32 {
        BACKEND_KIT
    }

    fn show(&mut self, w: &mut WindowCore, _s: &mut Sink) -> Result<()> {
        self.last.clear();
        let content = lower::lower(&w.scene)?;
        self.present(w, content)?;
        w.shown = true;
        Ok(())
    }

    fn close(&mut self, w: &mut WindowCore, _s: &mut Sink) {
        if w.shown {
            let _ = self.conn.send(&ToHost::Close { window: w.id }.to_line());
        }
        w.shown = false;
    }

    fn title_changed(&mut self, w: &mut WindowCore, _s: &mut Sink) -> Result<()> {
        if w.shown {
            let content = lower::lower(&w.scene)?;
            self.present(w, content)
        } else {
            Ok(())
        }
    }

    fn committed(&mut self, w: &mut WindowCore, _s: &mut Sink) -> Result<()> {
        // Even a hidden window must fit the Kit's limits: the error belongs to the patch that
        // broke them, not to a later `show`.
        let content = lower::lower(&w.scene)?;
        if w.shown {
            self.present(w, content)
        } else {
            Ok(())
        }
    }

    fn pump(&mut self, w: &mut WindowCore, s: &mut Sink) {
        let mine: Vec<FromHost> = {
            let mut q = lock(&self.conn.inbox.q);
            let (mine, keep): (Vec<_>, Vec<_>) = q.drain(..).partition(|e| window_of(e) == w.id);
            *q = keep.into();
            mine
        };
        s.seq = w.scene.last_seq();
        if mine.is_empty() && self.conn.inbox.lost.load(Ordering::Acquire) && w.shown {
            s.diagnostic(1, copy::KIT_LOST);
            w.shown = false;
        }
        for e in mine {
            self.deliver(w, s, e);
        }
    }

    fn trim(&mut self, _level: u32) {
        self.last = String::new();
    }
}

fn window_of(e: &FromHost) -> u32 {
    match e {
        FromHost::Clicked { window, .. }
        | FromHost::Closed { window }
        | FromHost::Edited { window, .. }
        | FromHost::Toggled { window, .. }
        | FromHost::Slid { window, .. }
        | FromHost::Chose { window, .. } => *window,
    }
}

impl KitWindow {
    fn deliver(&mut self, w: &mut WindowCore, s: &mut Sink, e: FromHost) {
        let kit = |widget: u32| lower::node_of(&w.scene, widget);
        match e {
            FromHost::Closed { .. } => {
                w.shown = false;
                s.emit(ev::CLOSE_REQUEST, 0, &[], &[]);
            }
            FromHost::Clicked { widget, .. } => {
                if let Some(r) = kit(widget).and_then(|n| receiver(&w.scene, n, ev::CLICK)) {
                    let mut head = [0u8; 16];
                    head[0..4].copy_from_slice(&1u32.to_le_bytes());
                    s.emit(ev::CLICK, w.scene.id(r).0, &head, &[]);
                }
            }
            FromHost::Edited { widget, text, .. } => {
                let Some(n) = kit(widget) else { return };
                let mut cur = w.scene.text[n.ix()];
                if let Some(true) = w.scene.set_str(&mut cur, &text) {
                    let old = w.scene.text[n.ix()];
                    w.scene.text[n.ix()] = cur;
                    w.scene.strings.release(old);
                    w.scene.version += 1;
                }
                // The host drew the edit already: remember it so an unchanged tree stays quiet.
                self.last.clear();
                if let Some(r) = receiver(&w.scene, n, ev::TEXT_CHANGED) {
                    let len = to_u32(text.len());
                    let mut head = Vec::with_capacity(12);
                    for v in [len, len, len] {
                        head.extend_from_slice(&v.to_le_bytes());
                    }
                    s.emit(ev::TEXT_CHANGED, w.scene.id(r).0, &head, text.as_bytes());
                }
            }
            FromHost::Toggled { widget, on, .. } => {
                let Some(n) = kit(widget) else { return };
                let cur = w.scene.state[n.ix()];
                w.scene.state[n.ix()] = if on {
                    cur | crate::scene::state::CHECKED
                } else {
                    cur & !crate::scene::state::CHECKED
                };
                self.last.clear();
                if let Some(r) = receiver(&w.scene, n, ev::TOGGLE) {
                    s.emit(
                        ev::TOGGLE,
                        w.scene.id(r).0,
                        &u32::from(on).to_le_bytes(),
                        &[],
                    );
                }
            }
            FromHost::Slid { widget, value, .. } => {
                let Some(n) = kit(widget) else { return };
                let v = value as f32;
                w.scene.put_prop(
                    n,
                    crate::scene::prop::id::SLIDER_VALUE,
                    &crate::scene::PropValue::new(&[v.to_bits()]),
                );
                self.last.clear();
                if let Some(r) = receiver(&w.scene, n, ev::SLIDER) {
                    s.emit(ev::SLIDER, w.scene.id(r).0, &v.to_le_bytes(), &[]);
                }
            }
            FromHost::Chose { .. } => {}
        }
        pipeline::ensure_layout(w);
    }
}
