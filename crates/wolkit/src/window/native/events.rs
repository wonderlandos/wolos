// SPDX-License-Identifier: BSD-2-Clause
//! What a native window does with each protocol event, and the [`Backend`] it is to the engine.

use super::client::Proto;
use super::{NativeWindow, copy, data, ime, menus, mime, sizing, theme};
use crate::event::kind as ev;
use crate::input::keys::KeyPress;
use crate::input::{BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, Request, edit, focus, keys, pointer};
use crate::scene::prop::id;
use crate::scene::{Idx, Kind, NIL, SceneRead, SceneWrite};
use crate::window::route::receiver;
use crate::window::{
    BACKEND_NATIVE, Backend, POINTER_DOWN, POINTER_LEAVE, POINTER_MOVE, POINTER_SCROLL, POINTER_UP,
    PointerInput, Sink, WINDOW_FIXED_SIZE, WindowCore, pipeline,
};
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::time::Duration;
use wayland_client::Proxy;
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;

/// Linux button codes.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

fn button(code: u32) -> u32 {
    match code {
        BTN_LEFT => BUTTON_LEFT,
        BTN_RIGHT => BUTTON_RIGHT,
        BTN_MIDDLE => BUTTON_MIDDLE,
        c => c.saturating_sub(0x10c),
    }
}

/// The cursor a node asks for: its `cursor` property, or the text beam over a text field.
fn cursor_of(scene: &dyn SceneRead, mut n: Idx) -> Shape {
    let shapes = [
        Shape::Default,
        Shape::Pointer,
        Shape::Text,
        Shape::Grab,
        Shape::Grabbing,
        Shape::NotAllowed,
    ];
    let mut guard = 0;
    while n != NIL && guard <= crate::scene::MAX_DEPTH + 1 {
        if let Some(v) = scene.prop(n, id::CURSOR) {
            return shapes
                .get(usize::try_from(v.w[0]).unwrap_or(0))
                .copied()
                .unwrap_or(Shape::Default);
        }
        if scene.kind(n) == Kind::TextInput {
            return Shape::Text;
        }
        n = scene.parent(n);
        guard += 1;
    }
    Shape::Default
}

impl NativeWindow {
    /// Every event that arrived, in order.
    pub(super) fn handle_all(&mut self, w: &mut WindowCore, s: &mut Sink) {
        let events = std::mem::take(&mut self.client.out);
        for e in events {
            self.handle(w, s, e);
        }
    }

    /// Apply one protocol event to the window (public for the injected-protocol tests).
    pub fn handle(&mut self, w: &mut WindowCore, s: &mut Sink, e: Proto) {
        match e {
            Proto::Configure(c, serial) => self.configure(w, s, c, serial),
            Proto::Close => s.emit(ev::CLOSE_REQUEST, 0, &[], &[]),
            Proto::FractionalScale(f) => {
                self.fractional = Some(f);
                self.rescale(w, s);
            }
            Proto::BufferScale(b) => {
                self.buffer_scale = b;
                self.rescale(w, s);
            }
            Proto::OutputsChanged => self.rescale(w, s),
            Proto::Decoration(server) => {
                if !server && self.server_side != Some(false) {
                    s.diagnostic(1, copy::CLIENT_SIDE);
                }
                self.server_side = Some(server);
            }
            Proto::FrameDone => self.frame_done(w, s),
            Proto::Presented { ns, refresh_ns } => self.pacer.presented(ns, refresh_ns),
            Proto::Discarded => self.pacer.discarded(),
            Proto::BufferReleased(id) => {
                self.buffers.released(id);
                if self.pacer.is_dirty() && !self.pacer.is_waiting() {
                    self.draw(w);
                }
            }
            Proto::PointerEnter { .. }
            | Proto::PointerLeave
            | Proto::PointerMotion { .. }
            | Proto::PointerButton { .. }
            | Proto::PointerAxis { .. }
            | Proto::PointerFrame => self.pointer_event(w, s, e),
            Proto::KeyboardEnter { serial } => {
                self.serial = serial;
                self.keyboard_focus = true;
            }
            Proto::KeyboardLeave => {
                self.keyboard_focus = false;
                self.stop_repeat();
            }
            Proto::Key { .. } => self.key_event(w, s, e),
            Proto::Modifiers => {}
            Proto::RepeatInfo { rate, delay } => self.repeat_info = (rate, delay),
            Proto::TextInputEnter
            | Proto::TextInputLeave
            | Proto::Preedit { .. }
            | Proto::CommitString(_)
            | Proto::DeleteSurrounding { .. }
            | Proto::TextInputDone(_) => self.text_input_event(w, s, e),
            Proto::Selection(_)
            | Proto::DragEnter { .. }
            | Proto::DragMotion { .. }
            | Proto::DragLeave
            | Proto::Drop
            | Proto::SourceSend { .. }
            | Proto::SourceCancelled => self.data_event(w, s, e),
            Proto::ActivationToken(token) => {
                if let (Some(a), Some(surf)) = (&self.g.activation, &self.surf) {
                    a.activate(token, &surf.surface);
                }
            }
        }
    }

    fn configure(&mut self, w: &mut WindowCore, s: &mut Sink, c: sizing::Configure, serial: u32) {
        let Some(surf) = &self.surf else {
            return;
        };
        surf.xdg.ack_configure(serial);
        let fixed = w.flags & WINDOW_FIXED_SIZE != 0;
        let (nw, nh) = if fixed {
            (w.width, w.height)
        } else {
            sizing::size_after(&c, (w.width, w.height), w.min, w.max)
        };
        self.activated = c.activated;
        let first = !self.configured;
        self.configured = true;
        if first || (nw, nh) != (w.width, w.height) {
            w.width = nw;
            w.height = nh;
            w.scale = self.scale();
            resize_event(w, s);
        }
        // A configure is answered with a commit carrying a buffer (the first one maps the window).
        self.pacer.reset();
        self.pacer.request();
        self.draw(w);
    }

    fn rescale(&mut self, w: &mut WindowCore, s: &mut Sink) {
        let scale = self.scale();
        if (scale - w.scale).abs() > f32::EPSILON {
            w.scale = scale;
            resize_event(w, s);
            if self.pacer.request() {
                self.draw(w);
            }
        }
    }

    /// A picture finished decoding since the last frame: draw again (its node showed the
    /// placeholder).
    fn pictures_ready(&mut self, w: &mut WindowCore) {
        if let Some(i) = &self.images {
            i.drain();
        }
        let g = crate::image::generation();
        if g == self.image_gen {
            return;
        }
        self.image_gen = g;
        w.scene.mark_dirty(0, crate::scene::dirty::PAINT);
        if w.shown && self.pacer.request() {
            self.draw(w);
        }
    }

    fn frame_done(&mut self, w: &mut WindowCore, s: &mut Sink) {
        let now = super::sys::monotonic_ns();
        let dt = if self.last_frame_ns == 0 {
            0
        } else {
            now.saturating_sub(self.last_frame_ns)
        };
        self.last_frame_ns = now;
        // Input that waited for the frame goes first (it reads the newest position).
        self.flush_pointer(w, s);
        let animating = !w.clock.is_empty();
        if animating && dt > 0 {
            w.step_anims(s, dt);
            self.pacer.request();
        }
        let ticks = w.scene.event_mask(0) & crate::event::bit(ev::FRAME_TICK) != 0;
        if ticks {
            let next = self.pacer.next_vblank(now);
            let mut head = [0u8; 8];
            head.copy_from_slice(&next.to_le_bytes());
            s.emit(ev::FRAME_TICK, 0, &head, &[]);
        }
        self.pacer.set_continuous(animating || ticks);
        if self.pacer.on_done() {
            self.draw(w);
        } else {
            self.last_frame_ns = 0;
        }
    }

    fn pointer_event(&mut self, w: &mut WindowCore, s: &mut Sink, e: Proto) {
        let mods = self.mods();
        let p = |kind: u32, x: f32, y: f32, button: u32| PointerInput {
            kind,
            x,
            y,
            button,
            mods,
        };
        let (last_x, last_y) = self.raw_pointer;
        let mut out = std::mem::take(&mut self.pending);
        match e {
            Proto::PointerEnter { serial, x, y } => {
                self.pointer_serial = serial;
                self.cursor_sent = None;
                self.raw_pointer = (x, y);
                self.input.coalesce.push(p(POINTER_MOVE, x, y, 0), &mut out);
            }
            Proto::PointerLeave => self
                .input
                .coalesce
                .push(p(POINTER_LEAVE, last_x, last_y, 0), &mut out),
            Proto::PointerMotion { x, y } => {
                self.raw_pointer = (x, y);
                self.input.coalesce.push(p(POINTER_MOVE, x, y, 0), &mut out);
            }
            Proto::PointerButton {
                serial,
                button: b,
                pressed,
            } => {
                self.serial = serial;
                let kind = if pressed { POINTER_DOWN } else { POINTER_UP };
                // The newest position, even if its move is still held for the frame.
                let (x, y) = self.raw_pointer;
                self.input.coalesce.push(p(kind, x, y, button(b)), &mut out);
            }
            Proto::PointerAxis { dx, dy } => self
                .input
                .coalesce
                .push(p(POINTER_SCROLL, dx, dy, 0), &mut out),
            _ => {}
        }
        self.pending = out;
        self.route_pending(w, s);
    }

    fn route_pending(&mut self, w: &mut WindowCore, s: &mut Sink) {
        let pending = std::mem::take(&mut self.pending);
        for p in &pending {
            if let Ok(r) = pointer::pointer(w, s, &mut self.input, *p) {
                self.request(w, s, r);
            }
        }
        self.pending = pending;
        self.pending.clear();
        self.update_cursor(w);
        self.after_input(w);
    }

    /// Send moves and scrolls held for the frame.
    pub(super) fn flush_pointer(&mut self, w: &mut WindowCore, s: &mut Sink) {
        if self.input.coalesce.is_empty() {
            return;
        }
        let mut out = std::mem::take(&mut self.pending);
        self.input.coalesce.flush(&mut out);
        self.pending = out;
        self.route_pending(w, s);
    }

    fn update_cursor(&mut self, w: &WindowCore) {
        let Some(dev) = &self.dev.cursor else {
            return;
        };
        let shape = cursor_of(&w.scene, w.hover);
        let code = u32::from(shape);
        if self.cursor_sent != Some(code) {
            dev.set_shape(self.pointer_serial, shape);
            self.cursor_sent = Some(code);
        }
    }

    fn key_event(&mut self, w: &mut WindowCore, s: &mut Sink, e: Proto) {
        let Proto::Key {
            serial,
            keysym,
            pressed,
            mods,
            text,
            repeats,
        } = e
        else {
            return;
        };
        self.serial = serial;
        // A key goes after the pointer state it follows.
        self.flush_pointer(w, s);
        if pressed && repeats {
            self.start_repeat(super::Held {
                keysym,
                mods,
                text: text.clone(),
            });
        } else if pressed || self.held.as_ref().is_some_and(|h| h.keysym == keysym) {
            self.stop_repeat();
        }
        let k = KeyPress {
            keysym,
            pressed,
            mods,
            repeat: false,
            text: &text,
        };
        let r = keys::key(w, s, &mut self.input, k);
        self.request(w, s, r);
        self.after_input(w);
    }

    fn start_repeat(&mut self, h: super::Held) {
        let (rate, delay) = self.repeat_info;
        self.held = Some(h);
        if let (Some(t), Ok(rate), Ok(delay)) = (
            &self.repeat_timer,
            u64::try_from(rate),
            u64::try_from(delay),
        ) && rate > 0
        {
            let every = Duration::from_micros(1_000_000 / rate.max(1));
            // A timer that cannot be set only means the key does not repeat.
            let _ = t.set(Some(Duration::from_millis(delay)), every);
        }
    }

    fn stop_repeat(&mut self) {
        self.held = None;
        if let Some(t) = &self.repeat_timer {
            let _ = t.set(None, Duration::ZERO);
        }
    }

    fn repeat_fired(&mut self, w: &mut WindowCore, s: &mut Sink) {
        let n = self
            .repeat_timer
            .as_ref()
            .map_or(0, super::sys::Timer::take);
        let Some(h) = self.held.clone() else {
            return;
        };
        // A late wakeup does not burst more than a few repeats.
        for _ in 0..n.min(4) {
            let k = KeyPress {
                keysym: h.keysym,
                pressed: true,
                mods: h.mods,
                repeat: true,
                text: &h.text,
            };
            let r = keys::key(w, s, &mut self.input, k);
            self.request(w, s, r);
        }
        self.after_input(w);
    }

    fn text_input_event(&mut self, w: &mut WindowCore, s: &mut Sink, e: Proto) {
        match e {
            Proto::TextInputEnter => self.ti.entered = true,
            Proto::TextInputLeave => {
                self.ti.leave();
                self.input.field.editor.ime_cancel();
            }
            Proto::Preedit { text, begin, end } => {
                let cursor = ime::preedit_cursor(&text, begin, end);
                edit::ime(w, &mut self.input, edit::Ime::Preedit { text, cursor });
            }
            Proto::CommitString(t) => edit::ime(w, &mut self.input, edit::Ime::Commit(t)),
            Proto::DeleteSurrounding { before, after } => {
                let (b, a) = (
                    usize::try_from(before).unwrap_or(0),
                    usize::try_from(after).unwrap_or(0),
                );
                edit::ime(
                    w,
                    &mut self.input,
                    edit::Ime::DeleteSurrounding {
                        before: b,
                        after: a,
                    },
                );
            }
            Proto::TextInputDone(serial) => {
                edit::ime_done(w, s, &mut self.input, serial);
                let want = self.want(w);
                let mut outs = std::mem::take(&mut self.outs);
                self.ti.done(serial, &want, &mut outs);
                self.outs = outs;
                self.send_ti();
                self.after_input(w);
                return;
            }
            _ => {}
        }
        self.sync_ime(w);
    }

    /// What the input method should know now.
    fn want(&self, w: &WindowCore) -> ime::Want {
        let on = self.input.field.node() != NIL && self.input.field.editor.ime_enabled();
        let rect = edit::cursor_rect(w, &self.input).map(|r| {
            let i = crate::num::round_i32;
            [i(r.x), i(r.y), i(r.w), i(r.h)]
        });
        ime::Want {
            on,
            surrounding: edit::surrounding(&self.input),
            rect,
            hint: ime::content::HINT_NONE,
            purpose: ime::content::PURPOSE_NORMAL,
        }
    }

    fn sync_ime(&mut self, w: &WindowCore) {
        let want = self.want(w);
        let mut outs = std::mem::take(&mut self.outs);
        self.ti.update(&want, &mut outs);
        self.outs = outs;
        self.send_ti();
    }

    fn send_ti(&mut self) {
        let Some(t) = &self.dev.text_input else {
            self.outs.clear();
            return;
        };
        for o in self.outs.drain(..) {
            match o {
                ime::Out::Enable => t.enable(),
                ime::Out::Disable => t.disable(),
                ime::Out::ContentType { hint, purpose } => {
                    use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
                        ContentHint, ContentPurpose,
                    };
                    let hint = ContentHint::from_bits_truncate(hint);
                    let purpose =
                        ContentPurpose::try_from(purpose).unwrap_or(ContentPurpose::Normal);
                    t.set_content_type(hint, purpose);
                }
                ime::Out::Surrounding {
                    text,
                    cursor,
                    anchor,
                } => t.set_surrounding_text(text, cursor, anchor),
                ime::Out::Cursor { x, y, w, h } => t.set_cursor_rectangle(x, y, w, h),
                ime::Out::Commit => t.commit(),
            }
        }
    }

    /// Do what routing asked the backend to do.
    fn request(&mut self, w: &mut WindowCore, s: &mut Sink, r: Request) {
        match r {
            Request::None => {}
            Request::Ime(_) => self.sync_ime(w),
            Request::Copy(text) => self.copy(text),
            Request::Paste => self.paste(w, s),
        }
    }

    fn copy(&mut self, text: String) {
        let (Some(m), Some(d)) = (&self.g.data, &self.dev.data) else {
            return;
        };
        if let Some(old) = self.dev.source.take() {
            old.destroy();
        }
        let src = m.create_data_source(&self.qh, ());
        for t in mime::OFFER {
            src.offer((*t).to_owned());
        }
        d.set_selection(Some(&src), self.serial);
        self.dev.source = Some(src);
        self.clip.ours = Some(text);
    }

    fn paste(&mut self, w: &mut WindowCore, s: &mut Sink) {
        if let Some(t) = self.clip.ours.clone() {
            // Our own copy: no round trip through the compositor (and no pipe to ourselves).
            edit::paste(w, s, &mut self.input, &t);
            return;
        }
        let Some(id) = self.clip.selection else {
            return;
        };
        let types = self.client.offer_types(id);
        let (Some(ty), Some(offer)) = (mime::text_type(&types), self.client.offer(id)) else {
            return;
        };
        let Ok((r, wfd)) = data::pair() else {
            return;
        };
        offer.receive(ty.to_owned(), wfd.as_fd());
        drop(wfd);
        self.clip.start_read(r);
    }

    fn data_event(&mut self, w: &mut WindowCore, s: &mut Sink, e: Proto) {
        match e {
            Proto::Selection(id) => {
                if let Some(old) = self.clip.selection.filter(|o| Some(*o) != id) {
                    self.client.drop_offer(old);
                }
                self.clip.selection = id;
            }
            Proto::SourceSend { mime: _, fd } => {
                if let Some(t) = self.clip.ours.clone() {
                    data::send(t, fd);
                }
            }
            Proto::SourceCancelled => {
                self.clip.ours = None;
                if let Some(src) = self.dev.source.take() {
                    src.destroy();
                }
            }
            Proto::DragEnter {
                serial,
                offer: Some(o),
                x,
                y,
            } => {
                self.clip.drag = Some(data::Drag {
                    offer: o,
                    serial,
                    x,
                    y,
                    accepted: None,
                    target: NIL,
                });
                self.drag_update(w);
            }
            Proto::DragMotion { x, y } => {
                if let Some(d) = &mut self.clip.drag {
                    d.x = x;
                    d.y = y;
                }
                self.drag_update(w);
            }
            Proto::DragLeave => {
                if let Some(d) = self.clip.drag.take() {
                    self.client.drop_offer(d.offer);
                }
            }
            Proto::Drop => self.dropped(w, s),
            _ => {}
        }
    }

    /// Accept the drag for the part under it if that part takes drops.
    fn drag_update(&mut self, w: &mut WindowCore) {
        let Some(d) = self.clip.drag.clone() else {
            return;
        };
        pipeline::ensure_layout(w);
        self.input.layers.refresh(&w.scene);
        let at = crate::input::hit::hit_test(&w.scene, &self.input.layers, d.x, d.y);
        let target = receiver(&w.scene, at, ev::DROP).unwrap_or(NIL);
        let types = self.client.offer_types(d.offer);
        let ty = (target != NIL)
            .then(|| mime::drop_type(&types).map(str::to_owned))
            .flatten();
        if let Some(offer) = self.client.offer(d.offer)
            && (target != d.target || ty != d.accepted)
        {
            offer.accept(d.serial, ty.clone());
            if offer.version() >= 3 {
                let act = if ty.is_some() {
                    DndAction::Copy | DndAction::Move
                } else {
                    DndAction::empty()
                };
                offer.set_actions(
                    act,
                    if ty.is_some() {
                        DndAction::Copy
                    } else {
                        DndAction::empty()
                    },
                );
            }
        }
        if let Some(cur) = &mut self.clip.drag {
            cur.target = target;
            cur.accepted = ty;
        }
    }

    fn dropped(&mut self, w: &mut WindowCore, s: &mut Sink) {
        let Some(d) = self.clip.drag.take() else {
            return;
        };
        if let (Some(ty), true) = (
            d.accepted.clone(),
            d.target != NIL && w.scene.is_live(d.target),
        ) && let Some(offer) = self.client.offer(d.offer)
            && let Ok((r, wfd)) = data::pair()
        {
            offer.receive(ty.clone(), wfd.as_fd());
            if offer.version() >= 3 {
                offer.finish();
            }
            drop(wfd);
            // The app reads at its pace: hand it a blocking descriptor.
            let _ = r.set_nonblocking(false);
            let (head, tail) = data::drop_payload(OwnedFd::from(r), d.x, d.y, &ty);
            s.emit(ev::DROP, w.scene.id(d.target).0, &head, &tail);
        }
        self.client.drop_offer(d.offer);
    }

    /// After input changed the scene: keep the input method and the frame in step.
    fn after_input(&mut self, w: &mut WindowCore) {
        self.sync_ime(w);
        if self.pacer.request() {
            self.draw(w);
        }
    }

    fn theme_changed(&mut self, w: &mut WindowCore, s: &mut Sink) {
        let Some(t) = &mut self.theme else {
            return;
        };
        let Some(a) = t.poll() else {
            return;
        };
        let text_scale = w.env.text_scale;
        w.env = a.env;
        w.env.text_scale = text_scale;
        w.style_version += 1;
        self.sync_glass(w);
        let mut head = [0u8; 12];
        head[0..4].copy_from_slice(&u32::from(w.env.dark).to_le_bytes());
        head[4..8].copy_from_slice(&w.env.accent_rgb.to_le_bytes());
        head[8..12].copy_from_slice(&w.env.flags().to_le_bytes());
        s.emit(ev::THEME_CHANGED, 0, &head, &[]);
        if self.pacer.request() {
            self.draw(w);
        }
    }

    /// The toplevel's app id carries the glass suffix while the root computes `backdrop: glass`
    /// and Reduce Transparency is off, so the compositor's `wol-glass` rule blurs what is behind
    /// the window. Sent before the first commit when the scene already says so, and again when
    /// it changes.
    pub(super) fn sync_glass(&mut self, w: &mut WindowCore) {
        pipeline::ensure_layout(w);
        let root = if w.scene.is_live(0) {
            w.styles.get(w.scene.computed(0)).paint.backdrop
        } else {
            crate::style::value::Backdrop::None
        };
        let glass = theme::wants_glass(root, &w.env);
        if glass == self.glass {
            return;
        }
        self.glass = glass;
        if let Some(surf) = &self.surf {
            surf.toplevel
                .set_app_id(theme::toplevel_app_id(&self.app_id, glass));
            self.flush();
        }
    }

    /// Publish the window's menus for the menu bar (see [`super::menus`]) when it has any, or
    /// withdraw them when it had and no longer does.
    pub(super) fn sync_menus(&mut self, w: &WindowCore) {
        let Some((hub, slot)) = self.a11y.clone() else {
            return;
        };
        let has = menus::has_menus(&w.scene);
        if !has && !self.menus_on {
            return;
        }
        let svc = if has {
            menus::shared(&self.app_id, hub)
        } else {
            menus::current()
        };
        let Some(svc) = svc else {
            return;
        };
        let m = if has {
            menus::collect(&w.scene)
        } else {
            Vec::new()
        };
        self.menus_on = !m.is_empty();
        self.menus_front = self.activated || self.keyboard_focus;
        svc.publish(slot, self.menus_front, m);
    }

    /// Build the accessibility tree and hand it to the hub (with change signals) while an
    /// assistive technology listens; in debug builds also report nameless controls once each.
    pub(super) fn publish_a11y(&mut self, w: &WindowCore, s: Option<&mut Sink>) {
        let Some((hub, slot)) = self.a11y.clone() else {
            return;
        };
        let active = hub.active();
        if !active && !(cfg!(debug_assertions) && s.is_some()) {
            return;
        }
        let tree = crate::a11y::build(
            &w.scene,
            &crate::a11y::Context {
                window: w.id,
                title: &w.title,
                active: self.activated || self.keyboard_focus,
                resizable: w.flags & WINDOW_FIXED_SIZE == 0,
                field: Some(&self.input.field),
            },
        );
        if let Some(s) = s {
            if self.linted.len() > 4096 {
                self.linted.clear();
            }
            for (id, msg) in crate::a11y::lint(&tree) {
                if self.linted.insert(id) {
                    s.diagnostic(1, &msg);
                }
            }
        }
        if !active {
            return;
        }
        let generation = hub.generation();
        let old = hub.publish(slot, tree.clone());
        if generation == self.a11y_gen
            && let Some(old) = old
        {
            crate::a11y::bridge::changes(&hub, slot, &old, &tree);
        }
        self.a11y_gen = generation;
    }

    /// Do what an assistive technology asked.
    fn a11y_actions(&mut self, w: &mut WindowCore, s: &mut Sink) {
        use crate::a11y::hub::Action;
        let Some((hub, slot)) = self.a11y.clone() else {
            return;
        };
        hub.drain_wake();
        let actions = hub.take_actions(slot);
        let republish = self.a11y_gen != hub.generation() && hub.active();
        if actions.is_empty() && !republish {
            return;
        }
        for (id, a) in actions {
            let i = crate::scene::NodeId(id).slot();
            if !w.scene.is_live(i) || w.scene.id(i).0 != id {
                continue;
            }
            match a {
                Action::Default => {
                    if !pointer::activate(w, s, i, 0)
                        && let Some(r) = receiver(&w.scene, i, ev::A11Y_ACTION)
                    {
                        s.emit(ev::A11Y_ACTION, w.scene.id(r).0, &0u32.to_le_bytes(), &[]);
                    }
                }
                Action::Focus => {
                    focus::set_focus(w, s, &mut self.input, i, true);
                    let r = edit::focus_changed(w, &mut self.input);
                    self.request(w, s, r);
                }
                Action::Caret(c) => self.a11y_select(w, i, c, c),
                Action::Select(a, b) => self.a11y_select(w, i, a, b),
                Action::Value(v) => pointer::set_slider(w, s, i, v as f32),
                Action::ScrollTo => keys::scroll_into_view(w, s, i),
            }
        }
        self.after_input(w);
        // A fresh export (an assistive technology just attached) gets the current tree now.
        self.publish_a11y(w, Some(s));
    }

    /// Caret or selection (character offsets) in the focused field.
    fn a11y_select(&mut self, w: &mut WindowCore, node: Idx, anchor: usize, head: usize) {
        if self.input.field.node() != node {
            return;
        }
        let text = self.input.field.editor.text();
        let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
        self.input
            .field
            .editor
            .set_selection(byte(anchor), byte(head));
        w.scene
            .mark_dirty(node, crate::scene::dirty::PAINT | crate::scene::dirty::A11Y);
    }

    fn pastes(&mut self, w: &mut WindowCore, s: &mut Sink) {
        let done = self.clip.poll();
        for t in done {
            edit::paste(w, s, &mut self.input, &t);
        }
    }
}

fn resize_event(w: &WindowCore, s: &mut Sink) {
    let mut head = [0u8; 12];
    head[0..4].copy_from_slice(&w.width.to_le_bytes());
    head[4..8].copy_from_slice(&w.height.to_le_bytes());
    head[8..12].copy_from_slice(&w.scale.to_bits().to_le_bytes());
    s.emit(ev::RESIZE, 0, &head, &[]);
}

impl Backend for NativeWindow {
    fn kind(&self) -> u32 {
        BACKEND_NATIVE
    }

    fn show(&mut self, w: &mut WindowCore, _s: &mut Sink) -> crate::error::Result<()> {
        if self.lost {
            return Err(crate::error::Error::unavailable(copy::LOST));
        }
        if self.surf.is_none() {
            // The app id the first commit carries already says whether the window wants glass.
            self.sync_glass(w);
            self.open_surface(w);
        }
        w.shown = true;
        Ok(())
    }

    fn close(&mut self, w: &mut WindowCore, _s: &mut Sink) {
        self.close_surface();
        w.shown = false;
    }

    fn title_changed(&mut self, w: &mut WindowCore, _s: &mut Sink) -> crate::error::Result<()> {
        if let Some(surf) = &self.surf {
            surf.toplevel.set_title(w.title.clone());
            self.flush();
        }
        Ok(())
    }

    fn limits_changed(&mut self, w: &mut WindowCore, _s: &mut Sink) -> crate::error::Result<()> {
        if let Some(surf) = &self.surf {
            let fixed = (w.flags & WINDOW_FIXED_SIZE != 0).then_some((w.width, w.height));
            let (min, max) = sizing::limits(w.min, w.max, fixed);
            surf.toplevel.set_min_size(min.0, min.1);
            surf.toplevel.set_max_size(max.0, max.1);
            surf.surface.commit();
            self.flush();
        }
        Ok(())
    }

    fn set_focus(&mut self, w: &mut WindowCore, s: &mut Sink, to: Idx, visible: bool) {
        // Through the input state, so the next `focus::sync` sees no change to report again.
        focus::set_focus(w, s, &mut self.input, to, visible);
        let r = edit::focus_changed(w, &mut self.input);
        self.request(w, s, r);
    }

    fn committed(&mut self, w: &mut WindowCore, s: &mut Sink) -> crate::error::Result<()> {
        if !self.subs.list.is_empty() {
            pipeline::ensure_layout(w);
            let scale = self.scale();
            self.subs.follow(w, s, scale);
        }
        if focus::sync(w, s, &mut self.input) {
            let r = edit::focus_changed(w, &mut self.input);
            self.request(w, s, r);
        }
        // A patch may have replaced the text of the field being edited.
        edit::refresh(w, &mut self.input);
        self.sync_ime(w);
        self.sync_glass(w);
        self.sync_menus(w);
        if w.shown && self.pacer.request() {
            self.draw(w);
        }
        self.publish_a11y(w, Some(s));
        Ok(())
    }

    fn pump(&mut self, w: &mut WindowCore, s: &mut Sink) {
        self.read();
        self.handle_all(w, s);
        if !self.subs.list.is_empty() {
            pipeline::ensure_layout(w);
            let scale = self.scale();
            self.subs.follow(w, s, scale);
        }
        if self.repeat_timer.is_some() {
            self.repeat_fired(w, s);
        }
        self.theme_changed(w, s);
        if self.menus_on && self.menus_front != (self.activated || self.keyboard_focus) {
            self.sync_menus(w);
        }
        self.pictures_ready(w);
        self.pastes(w, s);
        self.a11y_actions(w, s);
        // Moves wait for the frame while one is outstanding; an idle window routes them now.
        if !self.pacer.is_waiting() {
            self.flush_pointer(w, s);
        }
        for n in std::mem::take(&mut self.notes) {
            s.diagnostic(1, &n);
        }
        self.report_lost(s);
        self.flush();
    }

    fn fds(&self) -> Vec<RawFd> {
        let mut v = vec![self.shared.conn.backend().poll_fd().as_raw_fd()];
        if let Some(t) = &self.theme {
            v.extend(t.fds());
        }
        if let Some(t) = &self.repeat_timer {
            v.push(t.fd());
        }
        self.clip.fds(&mut v);
        if let Some((h, _)) = &self.a11y {
            v.push(h.wake_fd());
        }
        if let Some(i) = &self.images {
            v.push(i.fd());
        }
        v
    }

    fn render_rgba(
        &mut self,
        w: &mut WindowCore,
        out: &mut [u8],
    ) -> crate::error::Result<(u32, u32)> {
        pipeline::ensure_layout(w);
        let scale = self.scale();
        let (bw, bh) = (
            sizing::device(w.width, scale),
            sizing::device(w.height, scale),
        );
        let n = usize::try_from(bw).unwrap_or(0) * usize::try_from(bh).unwrap_or(0) * 4;
        if out.len() < n {
            return Err(crate::error::Error::invalid(format!(
                "The pixel buffer needs {n} bytes for a {bw} by {bh} window."
            )));
        }
        pipeline::render_rgba(w, bw, bh, scale, &mut out[..n]);
        Ok((bw, bh))
    }

    fn create_subsurface(
        &mut self,
        w: &mut WindowCore,
        node: Idx,
        _width: u32,
        _height: u32,
    ) -> crate::error::Result<crate::window::Subsurface> {
        // The size follows the node's layout (`SURFACE_RESIZED` reports changes).
        let g = self.create_gpu_surface(w, node)?;
        Ok(crate::window::Subsurface {
            id: g.id,
            display: g.display,
            surface: g.surface,
            width: g.width,
            height: g.height,
            scale: g.scale,
        })
    }

    fn destroy_subsurface(&mut self, id: u32) {
        self.destroy_gpu_surface(id);
    }

    fn trim(&mut self, level: u32) {
        if let Some(g) = &mut self.gpu {
            g.trim();
        }
        if level >= 1 && self.surf.is_none() {
            self.buffers.clear();
            self.rgba = Vec::new();
        }
    }
}
