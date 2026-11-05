// SPDX-License-Identifier: BSD-2-Clause
//! A window without a display: a [`WindowCore`] with the kit look, the shell's tokens and fonts
//! from memory, driven by input records and drawn by any [`crate::Renderer`]. The web page
//! (`crates/wolkit-web`) and the native tests use this same type, so what the browser shows is
//! what the tests check.
//!
//! The engine owns the behaviour of its parts (a switch flips itself, a slider moves, a field
//! edits); the page hands the resulting event records back to the caller, which plays the app.

use std::sync::{Arc, Mutex};

use shared::view::Node;
use wolkit::env::Environment;
use wolkit::event::EventQueue;
use wolkit::paint::list::DisplayList;
use wolkit::paint::palette::Palette;
use wolkit::paint::scene::{Lower, build_with};
use wolkit::scene::SceneRead;
use wolkit::scene::patch::{Patch, Writer};
use wolkit::text::fontdb::FontDb;
use wolkit::text::system::TextSystem;
use wolkit::window::headless::Headless;
use wolkit::window::{Backend, KeyInput, PointerInput, Sink, WindowCore, pipeline};

use crate::FrameIn;

/// The accent of the built-in themes (`crates/core/src/theme/builtin/wol-*.json`, `accent`).
fn theme_accent(dark: bool) -> u32 {
    // Relative to this file: crates/core/src/theme/builtin, the same JSON the shell themes from.
    const LIGHT: &str = include_str!("../../core/src/theme/builtin/wol-light.json");
    const DARK: &str = include_str!("../../core/src/theme/builtin/wol-dark.json");
    let v: serde_json::Value =
        serde_json::from_str(if dark { DARK } else { LIGHT }).unwrap_or_default();
    v["tokens"]["accent"]
        .as_str()
        .and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok())
        .unwrap_or(0x7452f5)
}

/// Event kinds as the page reports them (`wolkit::event::kind`), with the part's wire id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageEvent {
    pub kind: u16,
    pub node: u32,
}

pub struct Page {
    pub core: WindowCore,
    backend: Headless,
    events: EventQueue,
    pub list: DisplayList,
    seq: u32,
    /// What a click on each part of the design-system nodes means ([`crate::view::Bind`]).
    binds: Vec<crate::view::Bind>,
}

impl Page {
    /// A page `width` by `height` logical pixels at `scale`, light or dark, with `fonts` (font
    /// files in memory; the UI chain is built from their family names, Inter first).
    pub fn new(
        width: u32,
        height: u32,
        scale: f32,
        dark: bool,
        fonts: Vec<Vec<u8>>,
    ) -> Result<Page, String> {
        crate::view::register_icons();
        let env = Environment {
            dark,
            accent_rgb: theme_accent(dark),
            ..Environment::default()
        };
        let mut core = WindowCore::new(1, "wolkit", width, height, 0, env);
        core.scale = scale;
        let mut db = FontDb::new();
        for f in fonts {
            db.add_data(f).ok_or("A font file could not be read.")?;
        }
        let mut sys = TextSystem::from_db(db);
        sys.build_chain();
        core.text = Arc::new(Mutex::new(sys));
        core.style = Some(Arc::new(crate::view::style_table()?));
        core.style_version += 1;
        let mut page = Page {
            core,
            backend: Headless,
            events: EventQueue::new(),
            list: DisplayList::new(),
            seq: 0,
            binds: Vec::new(),
        };
        page.with(|w, b, s| b.show(w, s))
            .map_err(|e| e.to_string())?;
        Ok(page)
    }

    fn with<T>(&mut self, f: impl FnOnce(&mut WindowCore, &mut Headless, &mut Sink<'_>) -> T) -> T {
        let mut s = Sink {
            events: &mut self.events,
            window: 1,
            seq: self.seq,
        };
        f(&mut self.core, &mut self.backend, &mut s)
    }

    /// Applies a patch stream (what an app or the Crystal shard sends), atomically.
    pub fn apply(&mut self, bytes: &[u8]) -> Result<(), String> {
        let core = &mut self.core;
        let patch = Patch::parse(bytes, &mut core.scratch).map_err(|e| e.to_string())?;
        self.seq = patch.seq;
        core.scene
            .try_apply(&patch, &mut core.journal)
            .map_err(|e| e.to_string())?;
        core.scene.commit(&mut core.journal);
        self.with(|w, b, s| {
            w.start_anims(s, false);
            b.committed(w, s)
        })
        .map_err(|e| e.to_string())
    }

    /// Shows a `shared::view` tree ([`crate::view::raise`]).
    pub fn mount(&mut self, tree: &Node) -> Result<(), String> {
        let mut w = Writer::new();
        self.binds = crate::view::raise_bound(tree, &mut w);
        let bytes = w.finish(self.seq + 1).to_vec();
        self.apply(&bytes)
    }

    /// What a click on scene part `node` means to the app, when it is a part of the
    /// design-system nodes (a checkbox, an option, a chip, a tab, a disclosure header): these keep
    /// their state in the app's tree, so the page does not flip them itself.
    pub fn pick_for(&self, node: u32) -> Option<&crate::view::Pick> {
        self.binds.iter().find(|b| b.node == node).map(|b| &b.pick)
    }

    /// A new size (logical pixels) and scale.
    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        self.core.width = width;
        self.core.height = height;
        self.core.scale = scale;
    }

    /// Device pixels of the target.
    pub fn device_size(&self) -> (u32, u32) {
        let d = |v: u32| wolkit::num::round_u32((v as f32) * self.core.scale).max(1);
        (d(self.core.width), d(self.core.height))
    }

    /// A pointer record (window logical pixels, `wolkit::window::POINTER_*`).
    pub fn pointer(&mut self, p: PointerInput) -> Vec<PageEvent> {
        let _ = self.with(|w, b, s| b.inject_pointer(w, s, p));
        self.app_events()
    }

    /// A key record (`keysym`, X11 values, as `wolkit.h`).
    pub fn key(&mut self, k: KeyInput) -> Vec<PageEvent> {
        let _ = self.with(|w, b, s| b.inject_key(w, s, k));
        self.app_events()
    }

    /// Committed text (typing, an input method).
    pub fn text(&mut self, t: &str) -> Vec<PageEvent> {
        let _ = self.with(|w, b, s| b.inject_text(w, s, t));
        self.app_events()
    }

    /// Moves animations and transitions on by `ns`.
    pub fn advance(&mut self, ns: u64) {
        let _ = self.with(|w, b, s| b.advance_time(w, s, ns));
    }

    /// Drains the event records into (kind, part id) pairs.
    fn app_events(&mut self) -> Vec<PageEvent> {
        let mut buf = vec![0u8; wolkit::event::MAX_RECORD];
        let mut out = Vec::new();
        loop {
            let n = self.events.read(&mut buf);
            if n == 0 {
                break;
            }
            let mut at = 0;
            while at + wolkit::event::HEADER <= n {
                let len = usize::try_from(u32::from_le_bytes([
                    buf[at],
                    buf[at + 1],
                    buf[at + 2],
                    buf[at + 3],
                ]))
                .unwrap_or(0);
                let kind = u16::from_le_bytes([buf[at + 4], buf[at + 5]]);
                let node =
                    u32::from_le_bytes([buf[at + 12], buf[at + 13], buf[at + 14], buf[at + 15]]);
                out.push(PageEvent { kind, node });
                if len == 0 {
                    break;
                }
                at += len;
            }
        }
        out
    }

    /// Style, layout and lowering: the frame's display list (in `self.list`).
    pub fn build(&mut self) {
        // Layout, and every picture the page shows decoded first (icons: on the web build the
        // decode runs inline, natively on a worker), so a frame never depends on timing.
        pipeline::settle_images(&mut self.core);
        let palette = Palette::for_env(&self.core.env);
        let text = self.core.text.clone();
        let mut sys = wolkit::text::measure::lock(&text);
        build_with(
            &mut Lower {
                scene: &self.core.scene,
                styles: &self.core.styles,
                palette: &palette,
                text: &mut sys,
                cache: &mut self.core.text_cache,
            },
            Some(&self.core.canvases),
            &mut self.list,
        );
        self.list.finish();
    }

    /// A part's rectangle in window logical pixels (layout brought up to date first).
    pub fn node_rect(&mut self, node: u32) -> Option<[f32; 4]> {
        let id = wolkit::scene::NodeId(node);
        let i = id.slot();
        if !(self.core.scene.is_live(i) && self.core.scene.id(i) == id) {
            return None;
        }
        pipeline::ensure_layout(&mut self.core);
        let r = self.core.scene.rect(i);
        Some([r.x, r.y, r.w, r.h])
    }

    /// The look the page draws with: light or dark and the accent (0xRRGGBB). The page reads
    /// the system theme through its host (the shell's Kit host passes the shell's theme, the web
    /// loader the host page's theme message or `prefers-color-scheme`) and calls this on every
    /// change; colours are tokens resolved at paint time, so the next frame has the new look.
    /// Returns whether anything changed.
    pub fn set_theme(&mut self, dark: bool, accent_rgb: u32) -> bool {
        let env = &mut self.core.env;
        if env.dark == dark && env.accent_rgb == accent_rgb {
            return false;
        }
        env.dark = dark;
        env.accent_rgb = accent_rgb;
        self.core.style_version += 1;
        true
    }

    /// Applies the system theme the way native wolkit apps read it
    /// (`wolkit::window::native::theme::read`: the active theme file, its accent, Auto day and
    /// night). Feature `native` only; the shell's Kit host passes the shell's own theme and the
    /// web page the host page's. Returns whether anything changed.
    #[cfg(feature = "native")]
    pub fn follow_system_theme(&mut self, config_home: &std::path::Path) -> bool {
        use wolkit::window::native::{sys, theme};
        let a = theme::read(config_home, sys::local_time());
        self.set_theme(a.env.dark, a.env.accent_rgb)
    }

    /// The built-in theme's accent for light or dark (`crates/core/src/theme/builtin`).
    pub fn default_accent(dark: bool) -> u32 {
        theme_accent(dark)
    }

    fn live(&self, node: u32) -> Option<wolkit::scene::Idx> {
        let id = wolkit::scene::NodeId(node);
        let i = id.slot();
        (self.core.scene.is_live(i) && self.core.scene.id(i) == id).then_some(i)
    }

    /// Whether the part is checked (a switch that is on).
    pub fn checked(&self, node: u32) -> bool {
        self.live(node)
            .is_some_and(|i| self.core.scene.state(i) & wolkit::scene::state::CHECKED != 0)
    }

    /// A slider's value.
    pub fn slider_value(&self, node: u32) -> Option<f32> {
        let i = self.live(node)?;
        let v = self
            .core
            .scene
            .prop(i, wolkit::scene::prop::id::SLIDER_VALUE)?;
        Some(f32::from_bits(v.w[0]))
    }

    /// A part's text (a field's contents, a label).
    pub fn text_of(&self, node: u32) -> Option<String> {
        let i = self.live(node)?;
        Some(self.core.scene.text(i).to_owned())
    }

    /// Height the content needs: the bottom of the page's last part plus the page's bottom
    /// padding (the view sheet pads the page by the same step left and bottom), at the current
    /// width. None before a tree is mounted.
    pub fn content_height(&mut self) -> Option<f32> {
        pipeline::ensure_layout(&mut self.core);
        let page = self.live(crate::view::FIRST_STRUCT)?;
        let scene = &self.core.scene;
        let root = scene.rect(page);
        let mut c = scene.first_child(page);
        let (mut bottom, mut pad) = (root.y, None);
        while c != wolkit::scene::NIL {
            let r = scene.rect(c);
            bottom = bottom.max(r.y + r.h);
            pad.get_or_insert(r.x - root.x);
            c = scene.next_sibling(c);
        }
        Some(bottom - root.y + pad.unwrap_or(0.0))
    }

    /// The window's own background (the surface token).
    pub fn clear(&self) -> wolkit::paint::geom::Color {
        Palette::for_env(&self.core.env).background()
    }

    /// Builds the frame and hands it to `f` with the text system locked (a renderer call).
    pub fn draw<T>(&mut self, f: impl FnOnce(FrameIn<'_>) -> T) -> T {
        self.build();
        let (width, height) = self.device_size();
        let clear = self.clear();
        let text = self.core.text.clone();
        let mut sys = wolkit::text::measure::lock(&text);
        let sys = &mut *sys;
        f(FrameIn {
            list: &self.list,
            text: Some((&sys.db, &mut sys.glyphs)),
            width,
            height,
            scale: self.core.scale,
            clear,
        })
    }
}
