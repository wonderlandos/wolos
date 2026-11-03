// SPDX-License-Identifier: BSD-2-Clause
//! The engine: windows, their events and the dispatch loop, as a plain Rust API. `ffi/` wraps it
//! one-to-one. One lock guards the engine; the C ABI says "one thread at a time", the lock makes a
//! mistake a wait instead of a data race, and `retain`/`release` from other threads cannot race.

use crate::abi;
use crate::copy;
use crate::diag::{Counters, FrameRing};
use crate::env::Environment;
use crate::error::{Error, Result};
use crate::event::{self, EventQueue, kind as ev};
use crate::loop_::{self, Waker};
use crate::scene::patch::Patch;
use crate::scene::{Idx, NIL, NodeId, SceneRead};
use crate::window::{self, Backend, KeyInput, MAX_WINDOWS, PointerInput, Sink, WindowCore, dump, headless::Headless, kit, pipeline};
use std::sync::{Arc, Mutex, MutexGuard};

pub struct Window {
    pub core: WindowCore,
    pub backend: Box<dyn Backend>,
}

struct Inner {
    app_id: String,
    flags: u32,
    events: EventQueue,
    windows: Vec<Window>,
    next_id: u32,
    ever_had_window: bool,
    env: Environment,
    kit: Option<Arc<kit::Conn>>,
    kit_path: Option<std::path::PathBuf>,
    counters: Counters,
    frames: FrameRing,
}

pub struct Engine {
    inner: Mutex<Inner>,
    waker: Waker,
}

/// Window ids are never reused within an engine.
type WindowId = u32;

/// Reverse-DNS app id, as `shared::view::is_app_id`.
fn app_id_ok(s: &str) -> bool {
    shared::view::is_app_id(s)
}

impl Engine {
    pub fn new(app_id: &str, flags: u32) -> Result<Arc<Engine>> {
        if !app_id_ok(app_id) {
            return Err(Error::invalid(
                "An app id is reverse-DNS, like org.example.hello (letters, digits, '.', '-', '_').",
            ));
        }
        let waker = Waker::new().map_err(|_| Error::new(crate::error::Status::Internal, "The engine could not make its wake-up pipe."))?;
        Ok(Arc::new(Engine {
            inner: Mutex::new(Inner {
                app_id: app_id.to_owned(),
                flags,
                events: EventQueue::new(),
                windows: Vec::new(),
                next_id: 1,
                ever_had_window: false,
                env: Environment::default(),
                kit: None,
                kit_path: kit::socket_path(),
                counters: Counters::default(),
                frames: FrameRing::new(),
            }),
            waker,
        }))
    }

    /// Use this Kit socket instead of `$WOL_KIT_SOCKET` (tests and embedders).
    pub fn set_kit_socket(&self, path: &std::path::Path) {
        self.lock().kit_path = Some(path.to_owned());
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Readable while `dispatch` has work.
    pub fn fd(&self) -> i32 {
        self.waker.fd()
    }

    pub fn flags(&self) -> u32 {
        self.lock().flags
    }

    pub fn backend(&self) -> u32 {
        window::choose(self.lock().flags)
    }

    fn arm(&self, g: &Inner) {
        if !g.events.is_empty() {
            self.waker.wake();
        }
    }

    // ---- dispatch and events ----

    /// Wait up to `timeout_ms` (negative: forever, 0: poll) for work, pump the backends, and say
    /// what is pending.
    pub fn dispatch(&self, timeout_ms: i32) -> Result<u32> {
        let (mut fds, pending) = {
            let g = self.lock();
            let mut fds = vec![self.waker.fd()];
            for w in &g.windows {
                fds.extend(w.backend.fds());
            }
            (fds, !g.events.is_empty())
        };
        let mut ready = Vec::new();
        loop_::wait(&fds, if pending { 0 } else { timeout_ms }, &mut ready);
        fds.clear();
        self.waker.drain();
        let mut g = self.lock();
        let Inner { windows, events, counters, .. } = &mut *g;
        for w in windows.iter_mut() {
            let mut sink = Sink { events, window: w.core.id, seq: w.core.scene.last_seq() };
            w.backend.pump(&mut w.core, &mut sink);
        }
        counters.events = counters.events.max(0);
        let mut flags = 0;
        if !g.events.is_empty() {
            flags |= abi::DISPATCH_EVENTS;
            self.arm(&g);
        }
        if g.ever_had_window && g.windows.is_empty() {
            flags |= abi::DISPATCH_QUIT;
        }
        Ok(flags)
    }

    /// Whole event records into `out`.
    pub fn read_events(&self, out: &mut [u8]) -> usize {
        let mut g = self.lock();
        let n = g.events.read(out);
        self.arm(&g);
        n
    }

    // ---- engine-level calls ----

    pub fn info(&self, key: u32) -> u64 {
        let g = self.lock();
        match key {
            abi::INFO_BACKEND => u64::from(window::choose(g.flags)),
            abi::INFO_PATCH_FORMAT => u64::from(crate::scene::patch::FORMAT),
            abi::INFO_MAX_NODES => crate::scene::MAX_NODES as u64,
            abi::INFO_A11Y_ACTIVE => 0,
            abi::INFO_MAX_PATCH => crate::scene::patch::MAX_PATCH as u64,
            abi::INFO_MAX_WINDOWS => MAX_WINDOWS as u64,
            abi::INFO_MAX_EVENT_RECORD => event::MAX_RECORD as u64,
            abi::INFO_GPU => 0,
            _ => 0,
        }
    }

    pub fn trim(&self, level: u32) {
        let mut g = self.lock();
        for w in &mut g.windows {
            w.core.scene.trim();
            w.backend.trim(level);
        }
    }

    pub fn stats_json(&self) -> String {
        let g = self.lock();
        let (p50, p95, max) = g.frames.percentiles();
        let windows: Vec<_> = g
            .windows
            .iter()
            .map(|w| {
                serde_json::json!({
                    "id": w.core.id,
                    "nodes": w.core.scene.node_count(),
                    "strings": w.core.scene.strings().len(),
                    "memory_bytes": w.core.scene.memory(),
                    "shown": w.core.shown,
                })
            })
            .collect();
        serde_json::json!({
            "backend": window::choose(g.flags),
            "patches": g.counters.patches,
            "ops": g.counters.ops,
            "patch_errors": g.counters.patch_errors,
            "events_pending": g.events.len(),
            "frames": g.frames.total(),
            "frame_us": { "p50": p50, "p95": p95, "max": max },
            "windows": windows,
        })
        .to_string()
    }

    pub fn set_environment(&self, dark: bool, accent_rgb: u32, flags: u32) -> Result<()> {
        let mut g = self.lock();
        if g.flags & abi::ENGINE_HEADLESS == 0 {
            return Err(Error::unavailable(copy::HEADLESS_ONLY));
        }
        g.env.set(dark, accent_rgb, flags);
        let env = g.env;
        let Inner { windows, events, .. } = &mut *g;
        for w in windows.iter_mut() {
            w.core.env = env;
            w.core.style_version += 1;
            let mut head = [0u8; 12];
            head[0..4].copy_from_slice(&u32::from(dark).to_le_bytes());
            head[4..8].copy_from_slice(&(accent_rgb & 0x00ff_ffff).to_le_bytes());
            head[8..12].copy_from_slice(&flags.to_le_bytes());
            events.push(ev::THEME_CHANGED, w.core.id, 0, w.core.scene.last_seq(), &head, &[]);
        }
        self.arm(&g);
        Ok(())
    }

    // ---- windows ----

    pub fn create_window(&self, title: &str, width: u32, height: u32, flags: u32) -> Result<WindowId> {
        if title.len() > shared::view::MAX_TITLE || title.chars().any(char::is_control) {
            return Err(Error::invalid("A window title is one line of at most 256 bytes."));
        }
        if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
            return Err(Error::invalid("A window is between 1 and 16384 pixels each way."));
        }
        let mut g = self.lock();
        if g.windows.len() >= MAX_WINDOWS {
            return Err(Error::limit(copy::TOO_MANY_WINDOWS));
        }
        let backend: Box<dyn Backend> = match window::choose(g.flags) {
            window::BACKEND_HEADLESS => Box::new(Headless),
            window::BACKEND_KIT => {
                if g.kit.is_none() {
                    let conn = kit::connect_for(g.kit_path.as_deref(), &g.app_id, &self.waker)?;
                    g.kit = Some(conn);
                }
                match g.kit.clone() {
                    Some(c) => Box::new(kit::KitWindow::new(c)),
                    None => return Err(Error::unavailable(copy::KIT_GONE)),
                }
            }
            _ => return Err(Error::unavailable(copy::NO_DISPLAY)),
        };
        let id = g.next_id;
        g.next_id += 1;
        g.ever_had_window = true;
        let env = g.env;
        g.windows.push(Window { core: WindowCore::new(id, title, width, height, flags, env), backend });
        Ok(id)
    }

    fn with<R>(&self, id: WindowId, f: impl FnOnce(&mut Window, &mut EventQueue, &mut Counters, &mut FrameRing) -> Result<R>) -> Result<R> {
        let mut g = self.lock();
        let Inner { windows, events, counters, frames, .. } = &mut *g;
        let w = windows
            .iter_mut()
            .find(|w| w.core.id == id)
            .ok_or_else(|| Error::invalid(copy::WINDOW_GONE))?;
        let r = f(w, events, counters, frames);
        self.arm(&g);
        r
    }

    pub fn close_window(&self, id: WindowId) {
        let mut g = self.lock();
        let Inner { windows, events, .. } = &mut *g;
        if let Some(at) = windows.iter().position(|w| w.core.id == id) {
            let mut w = windows.remove(at);
            let mut sink = Sink { events, window: id, seq: w.core.scene.last_seq() };
            w.backend.close(&mut w.core, &mut sink);
        }
        self.arm(&g);
    }

    pub fn is_open(&self, id: WindowId) -> bool {
        self.lock().windows.iter().any(|w| w.core.id == id)
    }

    pub fn set_title(&self, id: WindowId, title: &str) -> Result<()> {
        if title.len() > shared::view::MAX_TITLE || title.chars().any(char::is_control) {
            return Err(Error::invalid("A window title is one line of at most 256 bytes."));
        }
        self.with(id, |w, events, _, _| {
            w.core.title = title.to_owned();
            let mut sink = Sink { events, window: id, seq: w.core.scene.last_seq() };
            w.backend.title_changed(&mut w.core, &mut sink)
        })
    }

    pub fn set_size_limits(&self, id: WindowId, min: (u32, u32), max: (u32, u32)) -> Result<()> {
        if (max.0 != 0 && max.0 < min.0) || (max.1 != 0 && max.1 < min.1) {
            return Err(Error::invalid("A window's largest size cannot be below its smallest."));
        }
        self.with(id, |w, events, _, _| {
            w.core.min = min;
            w.core.max = max;
            let mut sink = Sink { events, window: id, seq: w.core.scene.last_seq() };
            w.backend.limits_changed(&mut w.core, &mut sink)
        })
    }

    pub fn show(&self, id: WindowId) -> Result<()> {
        self.with(id, |w, events, _, _| {
            let mut sink = Sink { events, window: id, seq: w.core.scene.last_seq() };
            w.backend.show(&mut w.core, &mut sink)
        })
    }

    /// Hide the window (it stays open and keeps its scene).
    pub fn hide(&self, id: WindowId) -> Result<()> {
        self.with(id, |w, events, _, _| {
            let mut sink = Sink { events, window: id, seq: w.core.scene.last_seq() };
            w.backend.close(&mut w.core, &mut sink);
            Ok(())
        })
    }

    /// Validate and apply a patch, atomically: on any error the window is as it was.
    pub fn apply(&self, id: WindowId, bytes: &[u8]) -> Result<()> {
        self.with(id, |w, events, counters, frames| {
            let t0 = frames.now_ns();
            let patch = match Patch::parse(bytes, &mut w.core.scratch) {
                Ok(p) => p,
                Err(e) => {
                    counters.patch_errors += 1;
                    return Err(e);
                }
            };
            let core = &mut w.core;
            let applied = match core.scene.try_apply(&patch, &mut core.journal) {
                Ok(a) => a,
                Err(e) => {
                    counters.patch_errors += 1;
                    return Err(e);
                }
            };
            let mut sink = Sink { events, window: id, seq: patch.seq };
            if let Err(e) = w.backend.committed(core, &mut sink) {
                core.scene.rollback(&mut core.journal);
                counters.patch_errors += 1;
                return Err(e);
            }
            core.scene.commit(&mut core.journal);
            counters.patches += 1;
            counters.ops += u64::from(applied.ops);
            frames.record(crate::diag::Frame { begin_ns: t0, end_ns: frames.now_ns(), ..Default::default() });
            Ok(())
        })
    }

    pub fn read_text(&self, id: WindowId, node: u32, out: &mut [u8]) -> Result<usize> {
        self.with(id, |w, _, _, _| {
            let i = live_node(&w.core, node)?;
            let t = w.core.scene.text(i).as_bytes();
            let n = t.len().min(out.len());
            if t.len() <= out.len() {
                out[..n].copy_from_slice(&t[..n]);
            }
            Ok(t.len())
        })
    }

    pub fn set_focus(&self, id: WindowId, node: u32) -> Result<()> {
        self.with(id, |w, events, _, _| {
            let i = if node == 0 { NIL } else { live_node(&w.core, node)? };
            let mut sink = Sink { events, window: id, seq: w.core.scene.last_seq() };
            crate::window::headless::set_focus(&mut w.core, &mut sink, i, true);
            Ok(())
        })
    }

    pub fn size(&self, id: WindowId) -> Result<(u32, u32, f32)> {
        self.with(id, |w, _, _, _| Ok((w.core.width, w.core.height, w.core.scale)))
    }

    pub fn node_rect(&self, id: WindowId, node: u32) -> Result<[f32; 4]> {
        self.with(id, |w, _, _, _| {
            let i = live_node(&w.core, node)?;
            pipeline::ensure_layout(&mut w.core);
            let r = w.core.scene.rect(i);
            Ok([r.x, r.y, r.w, r.h])
        })
    }

    /// The id of the deepest node at the point, or 0 (the window) when none.
    pub fn hit_test(&self, id: WindowId, x: f32, y: f32) -> Result<u32> {
        self.with(id, |w, _, _, _| {
            pipeline::ensure_layout(&mut w.core);
            let i = crate::window::headless::hit_test(&w.core.scene, x, y);
            Ok(w.core.scene.id(i).0)
        })
    }

    pub fn dump(&self, id: WindowId, what: u32) -> Result<String> {
        if !(dump::SCENE..=dump::A11Y).contains(&what) {
            return Err(Error::invalid("A dump is the scene, the layout or the accessibility tree."));
        }
        self.with(id, |w, _, _, _| {
            if what == dump::LAYOUT {
                pipeline::ensure_layout(&mut w.core);
            }
            Ok(dump::dump(&w.core.scene, what))
        })
    }

    pub fn render_rgba(&self, id: WindowId, out: &mut [u8]) -> Result<(u32, u32)> {
        self.with(id, |w, _, _, _| w.backend.render_rgba(&mut w.core, out))
    }

    pub fn inject_pointer(&self, id: WindowId, p: PointerInput) -> Result<()> {
        self.with(id, |w, events, _, _| {
            let mut sink = Sink { events, window: id, seq: w.core.scene.last_seq() };
            w.backend.inject_pointer(&mut w.core, &mut sink, p)
        })
    }

    pub fn inject_key(&self, id: WindowId, k: KeyInput) -> Result<()> {
        self.with(id, |w, events, _, _| {
            let mut sink = Sink { events, window: id, seq: w.core.scene.last_seq() };
            w.backend.inject_key(&mut w.core, &mut sink, k)
        })
    }

    pub fn inject_text(&self, id: WindowId, t: &str) -> Result<()> {
        self.with(id, |w, events, _, _| {
            let mut sink = Sink { events, window: id, seq: w.core.scene.last_seq() };
            w.backend.inject_text(&mut w.core, &mut sink, t)
        })
    }

    pub fn advance_time(&self, id: WindowId, ns: u64) -> Result<()> {
        self.with(id, |w, events, _, _| {
            let mut sink = Sink { events, window: id, seq: w.core.scene.last_seq() };
            w.backend.advance_time(&mut w.core, &mut sink, ns)
        })
    }

    pub fn set_style_table(&self, id: WindowId, table: Option<Arc<crate::style::Table>>) -> Result<()> {
        self.with(id, |w, events, _, _| {
            w.core.style = table;
            w.core.style_version += 1;
            let mut sink = Sink { events, window: id, seq: w.core.scene.last_seq() };
            w.backend.committed(&mut w.core, &mut sink)
        })
    }
}

fn live_node(core: &WindowCore, raw: u32) -> Result<Idx> {
    let id = NodeId(raw);
    let i = id.slot();
    if core.scene.is_live(i) && core.scene.id(i) == id {
        Ok(i)
    } else {
        Err(Error::invalid(format!("Part {raw} is not a part of this window.")))
    }
}
