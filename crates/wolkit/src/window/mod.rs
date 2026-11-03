// SPDX-License-Identifier: BSD-2-Clause
//! Windows and their backends. A window is a [`WindowCore`] (the scene and everything backends
//! share) plus one [`Backend`] that decides where it appears:
//!
//! - [`headless`]: no display; tests, goldens and CI. Owns input injection.
//! - [`kit`]: lowers the scene to `shared::view` and talks to the Kit host over its socket.
//! - [`native`]: the engine's own Wayland window (WK-W4).
//!
//! One scene ABI, one conformance suite (`tests/conformance`) over all of them.

pub mod dump;
pub mod headless;
pub mod kit;
pub mod native;
pub mod pipeline;
pub mod route;

use crate::env::Environment;
use crate::error::{Error, Result};
use crate::event::EventQueue;
use crate::scene::ids::to_u32;
use crate::scene::{Idx, Journal, NIL, Scene, SceneRead, patch::Scratch};
use std::os::fd::RawFd;
use std::sync::Arc;

/// Windows per engine (plan section 2.8).
pub const MAX_WINDOWS: usize = 32;

/// `WOLKIT_BACKEND_*` as reported by `wolkit_engine_get_info(WOLKIT_INFO_BACKEND)`.
pub const BACKEND_HEADLESS: u32 = 1;
pub const BACKEND_KIT: u32 = 2;
pub const BACKEND_NATIVE: u32 = 3;

/// `WOLKIT_WINDOW_*` creation flags.
pub const WINDOW_FIXED_SIZE: u32 = 1;

/// Everything about one window that backends and the pipeline share.
pub struct WindowCore {
    pub id: u32,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub min: (u32, u32),
    pub max: (u32, u32),
    pub flags: u32,
    pub scene: Scene,
    pub journal: Journal,
    pub scratch: Scratch,
    /// Computed styles and the cascade memo (WK-W2).
    pub styles: crate::style::Styles,
    /// Layout measure cache and relayout boundaries (WK-W2).
    pub layout_cache: crate::layout::LayoutCache,
    /// Springs and tweens (`ANIM` ops) and the scratch they use.
    pub clock: crate::anim::Clock,
    pub anim_reqs: Vec<crate::scene::AnimReq>,
    pub anim_done: Vec<crate::anim::Done>,
    pub style: Option<Arc<crate::style::Table>>,
    /// Bumped when the style table or the environment changes, so the pipeline restyles.
    pub style_version: u64,
    pub shown: bool,
    pub env: Environment,
    /// The scene version, size and style version the layout was computed for.
    pub(crate) laid_out: Option<(u64, u32, u32, u32, u64)>,
    pub hover: Idx,
    pub pressed: Idx,
    /// Last pointer position (window space).
    pub pointer: (f32, f32),
    pub time_ns: u64,
}

impl WindowCore {
    pub fn new(
        id: u32,
        title: &str,
        width: u32,
        height: u32,
        flags: u32,
        env: Environment,
    ) -> Self {
        WindowCore {
            id,
            title: title.to_owned(),
            width,
            height,
            scale: 1.0,
            min: (0, 0),
            max: (0, 0),
            flags,
            scene: Scene::new(),
            journal: Journal::default(),
            scratch: Scratch::default(),
            styles: crate::style::Styles::default(),
            layout_cache: crate::layout::LayoutCache::new(),
            clock: crate::anim::Clock::new(),
            anim_reqs: Vec::new(),
            anim_done: Vec::new(),
            style: None,
            style_version: 0,
            shown: false,
            env,
            laid_out: None,
            hover: NIL,
            pressed: NIL,
            pointer: (0.0, 0.0),
            time_ns: 0,
        }
    }
}

impl WindowCore {
    /// Start the animations the last patch queued (they snap when `animate` is false or the
    /// person asked for reduced motion), and tell the app about the ones that finished at once.
    pub fn start_anims(&mut self, s: &mut Sink, animate: bool) {
        self.scene.take_anims(&mut self.anim_reqs);
        if self.anim_reqs.is_empty() {
            return;
        }
        let animate = animate && !self.env.reduce_motion;
        self.clock.start(
            &mut self.scene,
            &mut self.anim_reqs,
            self.time_ns,
            animate,
            &mut self.anim_done,
        );
        self.report_anims(s);
    }

    /// Move the clock to `ns` later in steps of at most one frame (a spring needs small steps).
    pub fn step_anims(&mut self, s: &mut Sink, ns: u64) {
        const FRAME_NS: u64 = 16_000_000;
        let target = self.time_ns.saturating_add(ns.min(3_600_000_000_000));
        while self.time_ns < target {
            self.time_ns = target.min(self.time_ns + FRAME_NS);
            if self.clock.is_empty() {
                self.time_ns = target;
                break;
            }
            self.clock
                .step(&mut self.scene, self.time_ns, &mut self.anim_done);
        }
        self.report_anims(s);
    }

    fn report_anims(&mut self, s: &mut Sink) {
        for d in std::mem::take(&mut self.anim_done) {
            let i = d.node.slot();
            if let Some(r) = route::receiver(&self.scene, i, crate::event::kind::ANIMATION_DONE) {
                s.emit(
                    crate::event::kind::ANIMATION_DONE,
                    self.scene.id(r).0,
                    &u32::from(d.prop).to_le_bytes(),
                    &[],
                );
            }
        }
    }
}

/// Where a backend puts the events it makes.
pub struct Sink<'a> {
    pub events: &'a mut EventQueue,
    pub window: u32,
    /// Sequence number of the last patch applied: events echo it.
    pub seq: u32,
}

impl Sink<'_> {
    pub fn emit(&mut self, kind: u16, node: u32, head: &[u8], tail: &[u8]) {
        self.events
            .push(kind, self.window, node, self.seq, head, tail);
    }

    /// A message for the developer: `level` 0 note, 1 warning, 2 error.
    pub fn diagnostic(&mut self, level: u32, text: &str) {
        let mut end = text.len().min(4096);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let mut head = [0u8; 8];
        head[..4].copy_from_slice(&level.to_le_bytes());
        head[4..].copy_from_slice(&(to_u32(end)).to_le_bytes());
        self.emit(
            crate::event::kind::DIAGNOSTIC,
            0,
            &head,
            &text.as_bytes()[..end],
        );
    }
}

/// The part of a window that differs between backends.
pub trait Backend: Send {
    /// `BACKEND_*`.
    fn kind(&self) -> u32;
    fn show(&mut self, w: &mut WindowCore, s: &mut Sink) -> Result<()>;
    fn close(&mut self, w: &mut WindowCore, s: &mut Sink);
    fn title_changed(&mut self, _w: &mut WindowCore, _s: &mut Sink) -> Result<()> {
        Ok(())
    }
    fn limits_changed(&mut self, _w: &mut WindowCore, _s: &mut Sink) -> Result<()> {
        Ok(())
    }
    /// A patch was applied (and is still undoable): present it, or refuse it with an error and the
    /// scene is rolled back. Also called after a style table swap.
    fn committed(&mut self, w: &mut WindowCore, s: &mut Sink) -> Result<()>;
    /// Called from `dispatch`: move what the backend received into events.
    fn pump(&mut self, _w: &mut WindowCore, _s: &mut Sink) {}
    /// Extra descriptors `dispatch` should wait on.
    fn fds(&self) -> Vec<RawFd> {
        Vec::new()
    }
    fn render_rgba(&mut self, _w: &mut WindowCore, _out: &mut [u8]) -> Result<(u32, u32)> {
        Err(Error::unavailable(crate::copy::HEADLESS_ONLY))
    }
    fn inject_pointer(
        &mut self,
        _w: &mut WindowCore,
        _s: &mut Sink,
        _p: PointerInput,
    ) -> Result<()> {
        Err(Error::unavailable(crate::copy::HEADLESS_ONLY))
    }
    fn inject_key(&mut self, _w: &mut WindowCore, _s: &mut Sink, _k: KeyInput) -> Result<()> {
        Err(Error::unavailable(crate::copy::HEADLESS_ONLY))
    }
    fn inject_text(&mut self, _w: &mut WindowCore, _s: &mut Sink, _t: &str) -> Result<()> {
        Err(Error::unavailable(crate::copy::HEADLESS_ONLY))
    }
    fn advance_time(&mut self, _w: &mut WindowCore, _s: &mut Sink, _ns: u64) -> Result<()> {
        Err(Error::unavailable(crate::copy::HEADLESS_ONLY))
    }
    /// Memory Saver: drop caches. 0 light, 1 hidden window, 2 critical.
    fn trim(&mut self, _level: u32) {}
}

/// `WOLKIT_POINTER_*` kinds.
pub const POINTER_MOVE: u32 = 0;
pub const POINTER_DOWN: u32 = 1;
pub const POINTER_UP: u32 = 2;
pub const POINTER_SCROLL: u32 = 3;
pub const POINTER_LEAVE: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerInput {
    pub kind: u32,
    pub x: f32,
    pub y: f32,
    pub button: u32,
    pub mods: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInput {
    pub keysym: u32,
    pub pressed: bool,
    pub mods: u32,
}

/// `WOLKIT_MOD_*` modifier bits (`Command` reaches apps as `CTRL`, like the rest of WolOS).
pub const MOD_SHIFT: u32 = 1;
pub const MOD_CTRL: u32 = 2;
pub const MOD_ALT: u32 = 4;
pub const MOD_SUPER: u32 = 8;

/// The backend a new window gets, from the engine's flags and the environment.
pub fn choose(flags: u32) -> u32 {
    if flags & crate::abi::ENGINE_HEADLESS != 0 {
        BACKEND_HEADLESS
    } else if flags & crate::abi::ENGINE_FORCE_KIT != 0 || !native::available() {
        BACKEND_KIT
    } else {
        BACKEND_NATIVE
    }
}
