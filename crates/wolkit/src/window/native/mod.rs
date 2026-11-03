// SPDX-License-Identifier: BSD-2-Clause
//! The native backend (plan sections 2.7 and 4.2), owned by WK-W4: the engine's own Wayland
//! window. Protocols: xdg-shell with a server-side decoration request (the compositor draws the
//! title bar and window lights), fractional-scale with viewporter, presentation-time,
//! linux-dmabuf feedback (for the GPU path), text-input-v3, the data device (clipboard, drag and
//! drop), cursor-shape and xdg-activation. Size limits and the decoration mode are sent before
//! the first commit, so the compositor's placement (`wol_place`, `wol_fit`) sees them.
//!
//! Each window has its own event queue on the process's one connection; its descriptors join the
//! engine's single wait set through [`Backend::fds`] (the socket, the theme watch, the key-repeat
//! timer, pastes being read), and [`Backend::pump`] does everything that arrived: protocol events
//! become input through `crate::input` (the same routing as the headless backend), configures
//! resize, frame callbacks pace drawing, the theme watch swaps the environment.
//!
//! Drawing is the CPU rasteriser into `wl_shm` buffers (`shm`); the GPU path (WK-W5) replaces the
//! present step and reads the dmabuf feedback kept in [`client::Dmabuf`].

pub mod client;
mod conn;
pub mod data;
pub mod dmabuf;
mod events;
pub mod gpu;
pub mod ime;
pub mod menus;
pub mod mime;
pub mod pacing;
pub mod seat;
pub mod shm;
pub mod sizing;
pub mod subsurface;
pub mod sys;
pub mod theme;

pub use conn::{
    Session, Shared, available, opted_in, session, set_opt_in, set_session, shared, use_socket,
};

use super::{Backend, PointerInput, Sink, WINDOW_FIXED_SIZE, WindowCore, pipeline};
use crate::error::{Error, Result};
use crate::event::kind as ev;
use crate::input::InputState;
use client::{Client, SeatCaps, ToplevelData};
use std::sync::Arc;
use wayland_client::globals::{GlobalList, registry_queue_init};
use wayland_client::protocol::{
    wl_compositor, wl_data_device, wl_data_device_manager, wl_data_source, wl_keyboard, wl_output,
    wl_pointer, wl_seat, wl_shm, wl_surface,
};
use wayland_client::{EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1;
use wayland_protocols::wp::presentation_time::client::wp_presentation;
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1;
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// Sentences of the native backend.
pub mod copy {
    pub const NO_SHELL: &str = "The compositor does not offer the window protocols this app needs.";
    pub const LOST: &str = "The compositor closed the connection, so the window is gone.";
    pub const NOT_SHOWN: &str = "Show the window before you put a GPU surface in it.";
    pub const GPU_OFF: &str = "This window draws on the CPU because the GPU path could not start:";
    pub const CLIENT_SIDE: &str =
        "The compositor asked this window to draw its own title bar, which the engine does not do.";
}

/// Whether the native backend is on in this build: always (the choice is `available`).
pub const BUILT: bool = true;

/// Globals the window binds.
struct Globals {
    compositor: wl_compositor::WlCompositor,
    subcompositor: Option<wayland_client::protocol::wl_subcompositor::WlSubcompositor>,
    shm: wl_shm::WlShm,
    wm_base: xdg_wm_base::XdgWmBase,
    decoration: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    fractional: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    presentation: Option<wp_presentation::WpPresentation>,
    text_input: Option<zwp_text_input_manager_v3::ZwpTextInputManagerV3>,
    data: Option<wl_data_device_manager::WlDataDeviceManager>,
    cursor: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    activation: Option<xdg_activation_v1::XdgActivationV1>,
    seat: Option<wl_seat::WlSeat>,
    seat_caps: Arc<SeatCaps>,
    _dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
}

/// Per-seat devices.
#[derive(Default)]
struct Devices {
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    cursor: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    text_input: Option<zwp_text_input_v3::ZwpTextInputV3>,
    data: Option<wl_data_device::WlDataDevice>,
    source: Option<wl_data_source::WlDataSource>,
}

/// The surface and its roles while the window is shown.
struct Surf {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
    decoration: Option<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1>,
    fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    viewport: Option<wp_viewport::WpViewport>,
}

/// A key held down that repeats.
#[derive(Clone, Debug)]
struct Held {
    keysym: u32,
    mods: u32,
    text: String,
}

pub struct NativeWindow {
    shared: Arc<Shared>,
    queue: EventQueue<Client>,
    qh: QueueHandle<Client>,
    client: Client,
    g: Globals,
    dev: Devices,
    surf: Option<Surf>,
    /// The app's own id (without the glass suffix).
    app_id: String,
    /// Whether the toplevel's app id carries the glass suffix now (the root computes
    /// `backdrop: glass` and Reduce Transparency is off).
    glass: bool,
    /// This window published menus last time, and whether it was in front.
    menus_on: bool,
    menus_front: bool,
    pub input: InputState,
    pub pacer: pacing::FramePacer,
    buffers: shm::Buffers,
    ti: ime::TextInput,
    clip: data::Clipboard,
    theme: Option<theme::Watch>,
    repeat_timer: Option<sys::Timer>,
    held: Option<Held>,
    /// Keys per second and delay in milliseconds.
    repeat_info: (i32, i32),
    fractional: Option<f32>,
    buffer_scale: i32,
    configured: bool,
    activated: bool,
    keyboard_focus: bool,
    /// Last input serial (for the selection and activation).
    serial: u32,
    pointer_serial: u32,
    cursor_sent: Option<u32>,
    pub server_side: Option<bool>,
    rgba: Vec<u8>,
    pending: Vec<PointerInput>,
    /// Where the pointer is now, moves held for the frame included.
    raw_pointer: (f32, f32),
    lost: bool,
    lost_reported: bool,
    last_frame_ns: u64,
    outs: Vec<ime::Out>,
    /// The accessibility hub and this window's slot in it.
    a11y: Option<(Arc<crate::a11y::hub::Hub>, u32)>,
    /// The hub generation the last published tree belongs to.
    a11y_gen: u64,
    /// Nameless controls already reported.
    linted: std::collections::HashSet<u32>,
    /// The GPU present path, once opened; `None` draws on the CPU into `wl_shm`.
    gpu: Option<gpu::GpuPath>,
    /// Draw on the CPU only (`WOLKIT_ENGINE_FORCE_CPU`, or the GPU path failed once).
    cpu_only: bool,
    /// Sentences for the app, reported as warnings on the next pump.
    notes: Vec<String>,
    /// GPU surfaces of `surface` nodes (`wolkit_surface_*`).
    subs: subsurface::Subs,
}

/// A GPU surface made for a `surface` node: its id, the pointers Vulkan WSI takes, and its size
/// in device pixels at `scale`.
#[derive(Clone, Copy, Debug)]
pub struct GpuSurface {
    pub id: u32,
    pub display: *mut std::ffi::c_void,
    pub surface: *mut std::ffi::c_void,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

// Releases the window's slot in the accessibility hub (its tree stops being exported).
impl Drop for NativeWindow {
    fn drop(&mut self) {
        // The presenter goes first: it refers to the surface by pointer.
        self.gpu = None;
        if let Some((h, slot)) = &self.a11y {
            if self.menus_on
                && let Some(svc) = menus::current()
            {
                svc.remove(*slot);
            }
            h.unregister(*slot);
        }
    }
}

fn bind_opt<I, U>(
    g: &GlobalList,
    qh: &QueueHandle<Client>,
    v: std::ops::RangeInclusive<u32>,
    u: U,
) -> Option<I>
where
    I: Proxy + 'static,
    U: Send + Sync + 'static,
    Client: wayland_client::Dispatch<I, U>,
{
    g.bind(qh, v, u).ok()
}

/// Open a native window's connection side (globals, seat, theme watch) on the shared display.
/// `gpu`: try the GPU present path (false for `WOLKIT_ENGINE_FORCE_CPU`).
pub fn create(app_id: &str, gpu: bool) -> Result<Box<dyn Backend>> {
    let shared = conn::shared()?;
    let mut w = NativeWindow::new(shared, app_id, &wol_core::theme::config_home())?;
    w.cpu_only = !gpu;
    Ok(Box::new(w))
}

impl NativeWindow {
    /// A window on `shared` that exports its accessibility tree through the process's hub.
    pub fn new(
        shared: Arc<Shared>,
        app_id: &str,
        config_home: &std::path::Path,
    ) -> Result<NativeWindow> {
        let hub = crate::a11y::hub::hub(app_id, crate::a11y::bridge::start);
        Self::with_hub(shared, app_id, config_home, hub)
    }

    /// The same with a given hub (tests: a private one, or none).
    pub fn with_hub(
        shared: Arc<Shared>,
        app_id: &str,
        config_home: &std::path::Path,
        hub: Option<Arc<crate::a11y::hub::Hub>>,
    ) -> Result<NativeWindow> {
        let lost = |_| Error::unavailable(copy::LOST);
        let (globals, mut queue) = registry_queue_init::<Client>(&shared.conn)
            .map_err(|_| Error::unavailable(copy::LOST))?;
        let qh = queue.handle();
        let need = |_| Error::unavailable(copy::NO_SHELL);
        let caps = Arc::new(SeatCaps::default());
        let g = Globals {
            compositor: globals.bind(&qh, 4..=6, ()).map_err(need)?,
            subcompositor: bind_opt(&globals, &qh, 1..=1, ()),
            shm: globals.bind(&qh, 1..=1, ()).map_err(need)?,
            wm_base: globals.bind(&qh, 1..=6, ()).map_err(need)?,
            decoration: bind_opt(&globals, &qh, 1..=1, ()),
            fractional: bind_opt(&globals, &qh, 1..=1, ()),
            viewporter: bind_opt(&globals, &qh, 1..=1, ()),
            presentation: bind_opt(&globals, &qh, 1..=1, ()),
            text_input: bind_opt(&globals, &qh, 1..=1, ()),
            data: bind_opt(&globals, &qh, 1..=3, ()),
            cursor: bind_opt(&globals, &qh, 1..=1, ()),
            activation: bind_opt(&globals, &qh, 1..=1, ()),
            seat: bind_opt(&globals, &qh, 1..=8, caps.clone()),
            seat_caps: caps,
            _dmabuf: bind_opt::<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _>(&globals, &qh, 4..=4, ()),
        };
        globals.contents().with_list(|list| {
            for gl in list
                .iter()
                .filter(|gl| gl.interface == wl_output::WlOutput::interface().name)
            {
                let _: wl_output::WlOutput =
                    globals.registry().bind(gl.name, gl.version.min(4), &qh, ());
            }
        });
        if let Some(d) = &g._dmabuf {
            d.get_default_feedback(&qh, ());
        }
        let mut client = Client::new(&conn::session().locale);
        queue.roundtrip(&mut client).map_err(lost)?;
        let mut w = NativeWindow {
            shared,
            queue,
            qh,
            client,
            g,
            dev: Devices::default(),
            surf: None,
            app_id: theme::base_app_id(app_id).to_owned(),
            glass: false,
            menus_on: false,
            menus_front: false,
            input: InputState::new(),
            pacer: pacing::FramePacer::default(),
            buffers: shm::Buffers::default(),
            ti: ime::TextInput::default(),
            clip: data::Clipboard::default(),
            theme: theme::Watch::new(config_home).ok(),
            repeat_timer: sys::Timer::new().ok(),
            held: None,
            repeat_info: (25, 400),
            fractional: None,
            buffer_scale: 0,
            configured: false,
            activated: false,
            keyboard_focus: false,
            serial: 0,
            pointer_serial: 0,
            cursor_sent: None,
            server_side: None,
            rgba: Vec::new(),
            pending: Vec::new(),
            raw_pointer: (0.0, 0.0),
            lost: false,
            lost_reported: false,
            last_frame_ns: 0,
            outs: Vec::new(),
            a11y: hub.map(|h| {
                let slot = h.register();
                (h, slot)
            }),
            a11y_gen: u64::MAX,
            linted: std::collections::HashSet::new(),
            gpu: None,
            cpu_only: true,
            notes: Vec::new(),
            subs: subsurface::Subs::default(),
        };
        w.buffers.choose_format(&w.client.shm_formats);
        w.devices();
        w.flush();
        Ok(w)
    }

    /// Make the devices the seat has (after the first roundtrip told its capabilities).
    fn devices(&mut self) {
        let Some(seat) = &self.g.seat else {
            return;
        };
        let caps = self.g.seat_caps.0.lock().map_or(0, |c| *c);
        let qh = &self.qh;
        if caps & u32::from(wl_seat::Capability::Pointer) != 0 && self.dev.pointer.is_none() {
            let p = seat.get_pointer(qh, ());
            self.dev.cursor = self.g.cursor.as_ref().map(|c| c.get_pointer(&p, qh, ()));
            self.dev.pointer = Some(p);
        }
        if caps & u32::from(wl_seat::Capability::Keyboard) != 0 && self.dev.keyboard.is_none() {
            self.dev.keyboard = Some(seat.get_keyboard(qh, ()));
        }
        if self.dev.text_input.is_none() {
            self.dev.text_input = self
                .g
                .text_input
                .as_ref()
                .map(|m| m.get_text_input(seat, qh, ()));
        }
        if self.dev.data.is_none() {
            self.dev.data = self
                .g
                .data
                .as_ref()
                .map(|m| m.get_data_device(seat, qh, ()));
        }
    }

    fn flush(&mut self) {
        if self.shared.conn.flush().is_err() {
            self.lost = true;
        }
    }

    /// The dmabuf feedback seen so far (for the GPU path).
    pub fn dmabuf(&self) -> &client::Dmabuf {
        &self.client.dmabuf
    }

    /// The scale the buffers are drawn at.
    pub fn scale(&self) -> f32 {
        match (
            self.fractional,
            self.surf.as_ref().and_then(|s| s.viewport.as_ref()),
        ) {
            (Some(f), Some(_)) if f > 0.0 => f,
            _ => {
                let s = if self.buffer_scale > 0 {
                    self.buffer_scale
                } else {
                    self.client.output_scale()
                };
                s.max(1) as f32
            }
        }
    }

    fn open_surface(&mut self, w: &WindowCore) {
        let qh = &self.qh;
        let surface = self.g.compositor.create_surface(qh, ());
        let data = Arc::new(ToplevelData::default());
        let xdg = self.g.wm_base.get_xdg_surface(&surface, qh, data.clone());
        let toplevel = xdg.get_toplevel(qh, data);
        toplevel.set_title(w.title.clone());
        toplevel.set_app_id(theme::toplevel_app_id(&self.app_id, self.glass));
        let fixed = (w.flags & WINDOW_FIXED_SIZE != 0).then_some((w.width, w.height));
        let (min, max) = sizing::limits(w.min, w.max, fixed);
        toplevel.set_min_size(min.0, min.1);
        toplevel.set_max_size(max.0, max.1);
        let decoration = self.g.decoration.as_ref().map(|m| {
            let d = m.get_toplevel_decoration(&toplevel, qh, ());
            d.set_mode(zxdg_toplevel_decoration_v1::Mode::ServerSide);
            d
        });
        let fractional = self
            .g
            .fractional
            .as_ref()
            .map(|m| m.get_fractional_scale(&surface, qh, ()));
        let viewport = self
            .g
            .viewporter
            .as_ref()
            .map(|v| v.get_viewport(&surface, qh, ()));
        // The first commit carries no buffer: it asks for the first configure.
        surface.commit();
        self.surf = Some(Surf {
            surface,
            xdg,
            toplevel,
            decoration,
            fractional,
            viewport,
        });
        if let (Some(a), Some(token), Some(s)) = (
            &self.g.activation,
            conn::take_activation_token(),
            &self.surf,
        ) {
            a.activate(token, &s.surface);
        }
        self.flush();
    }

    fn close_surface(&mut self) {
        // The presenter refers to the surface: it goes before the surface is destroyed, and so
        // do the subsurfaces, whose parent it is.
        self.gpu = None;
        self.subs.clear();
        if let Some(s) = self.surf.take() {
            if let Some(d) = s.decoration {
                d.destroy();
            }
            if let Some(f) = s.fractional {
                f.destroy();
            }
            if let Some(v) = s.viewport {
                v.destroy();
            }
            s.toplevel.destroy();
            s.xdg.destroy();
            s.surface.destroy();
        }
        self.buffers.clear();
        self.configured = false;
        self.pacer.reset();
        self.flush();
    }

    /// The `wl_display *` and this window's `wl_surface *`, for Vulkan WSI (the GPU present path
    /// and `wolkit_surface_*`), while the window is shown.
    pub fn wsi_handles(&self) -> Option<(*mut std::ffi::c_void, *mut std::ffi::c_void)> {
        let surf = self.surf.as_ref()?;
        let d = self
            .shared
            .conn
            .backend()
            .display_ptr()
            .cast::<std::ffi::c_void>();
        let p = surf.surface.id().as_ptr().cast::<std::ffi::c_void>();
        (!d.is_null() && !p.is_null()).then_some((d, p))
    }

    /// A GPU surface for `node` (a `surface` node) while the window is shown.
    pub fn create_gpu_surface(
        &mut self,
        w: &mut WindowCore,
        node: crate::scene::Idx,
    ) -> Result<GpuSurface> {
        pipeline::ensure_layout(w);
        let scale = self.scale();
        let Some(surf) = &self.surf else {
            return Err(Error::unavailable(copy::NOT_SHOWN));
        };
        let made = self
            .subs
            .create(&self.g, &surf.surface, &self.qh, w, node, scale)
            .map_err(Error::invalid)?;
        let display = self
            .shared
            .conn
            .backend()
            .display_ptr()
            .cast::<std::ffi::c_void>();
        let surface = made.surface.id().as_ptr().cast::<std::ffi::c_void>();
        self.flush();
        Ok(GpuSurface {
            id: made.id,
            display,
            surface,
            width: made.size.0,
            height: made.size.1,
            scale,
        })
    }

    pub fn destroy_gpu_surface(&mut self, id: u32) {
        self.subs.destroy(id);
        self.flush();
    }

    /// Whether frames go through the GPU present path now.
    pub fn on_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    fn open_gpu(&mut self, bw: u32, bh: u32) {
        if self.cpu_only || self.gpu.is_some() {
            return;
        }
        let Some((d, p)) = self.wsi_handles() else {
            return;
        };
        // SAFETY: `d` is the display of `self.shared`, which this window holds for its whole life;
        // `p` is `self.surf`'s surface. The path is dropped before the surface is destroyed
        // (`close_surface`) and before the connection (`Drop`), as `GpuPath::new` requires.
        match unsafe { gpu::GpuPath::new(d, p, bw, bh) } {
            Ok(g) => self.gpu = Some(g),
            Err(e) => {
                self.cpu_only = true;
                self.notes.push(format!("{} {}", copy::GPU_OFF, e.message));
            }
        }
    }

    /// The state a commit carries besides the buffer: the scale, the frame callback, feedback.
    fn surface_state(&self, w: &WindowCore, scale: f32) {
        let Some(surf) = &self.surf else {
            return;
        };
        let surface = &surf.surface;
        match &surf.viewport {
            Some(v) => {
                surface.set_buffer_scale(1);
                v.set_destination(
                    i32::try_from(w.width).unwrap_or(i32::MAX),
                    i32::try_from(w.height).unwrap_or(i32::MAX),
                );
            }
            None => surface.set_buffer_scale(crate::num::round_i32(scale).max(1)),
        }
        surface.frame(&self.qh, ());
        if let Some(p) = &self.g.presentation {
            p.feedback(surface, &self.qh, ());
        }
    }

    /// Draw the window and commit it with a frame callback: through the GPU when that path is
    /// open, else on the CPU into a `wl_shm` buffer.
    fn draw(&mut self, w: &mut WindowCore) {
        if self.surf.is_none() || !self.configured || self.lost {
            return;
        }
        let begin = sys::monotonic_ns();
        let scale = self.scale();
        if (w.scale - scale).abs() > f32::EPSILON {
            w.scale = scale;
        }
        pipeline::ensure_layout(w);
        let (bw, bh) = (
            sizing::device(w.width, scale),
            sizing::device(w.height, scale),
        );
        self.open_gpu(bw, bh);
        if let Some(mut g) = self.gpu.take() {
            self.surface_state(w, scale);
            match g.draw(w, bw, bh, scale) {
                Ok(p) => {
                    if p == crate::gpu::present::Presented::Skipped
                        && let Some(surf) = &self.surf
                    {
                        // Nothing to show: the commit still carries the frame callback.
                        surf.surface.commit();
                    }
                    self.gpu = Some(g);
                    pipeline::frame(w);
                    self.finish_frame(w, begin, bw, bh);
                    return;
                }
                Err(e) => {
                    // The presenter is dropped here, which frees the surface for `wl_shm`.
                    drop(g);
                    self.cpu_only = true;
                    self.notes.push(format!("{} {}", copy::GPU_OFF, e.message));
                }
            }
        }
        let n = usize::try_from(bw).unwrap_or(0) * usize::try_from(bh).unwrap_or(0) * 4;
        self.rgba.resize(n, 0);
        pipeline::render_rgba(w, bw, bh, scale, &mut self.rgba);
        pipeline::frame(w);
        let Some(buf) = self.buffers.put(&self.g.shm, &self.qh, bw, bh, &self.rgba) else {
            // Every buffer is with the compositor: draw again when one comes back.
            self.pacer.request();
            return;
        };
        let Some(surf) = &self.surf else {
            return;
        };
        surf.surface.attach(Some(&buf), 0, 0);
        let (iw, ih) = (
            i32::try_from(bw).unwrap_or(i32::MAX),
            i32::try_from(bh).unwrap_or(i32::MAX),
        );
        surf.surface.damage_buffer(0, 0, iw, ih);
        self.surface_state(w, scale);
        if let Some(surf) = &self.surf {
            surf.surface.commit();
        }
        self.finish_frame(w, begin, bw, bh);
    }

    fn finish_frame(&mut self, w: &WindowCore, begin: u64, bw: u32, bh: u32) {
        self.flush();
        self.publish_a11y(w, None);
        self.pacer.drew(pacing::FrameRecord {
            begin_ns: begin,
            end_ns: sys::monotonic_ns(),
            damage_px: u64::from(bw) * u64::from(bh),
            presented_ns: 0,
        });
    }

    /// Read the socket without blocking and run the handlers.
    fn read(&mut self) {
        if let Some(guard) = self.queue.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => self.lost = true,
            }
        }
        if self.queue.dispatch_pending(&mut self.client).is_err() {
            self.lost = true;
        }
    }

    fn report_lost(&mut self, s: &mut Sink) {
        if self.lost && !self.lost_reported {
            self.lost_reported = true;
            s.diagnostic(2, copy::LOST);
            s.emit(ev::CLOSE_REQUEST, 0, &[], &[]);
        }
    }

    fn mods(&self) -> u32 {
        self.client.keyboard.mods()
    }
}
