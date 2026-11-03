// SPDX-License-Identifier: BSD-2-Clause
//! The native backend (`wolkit::window::native`, WK-W4) on the wire, against a small compositor
//! that runs in the test process (`wayland-server`, on a socket pair). The forge image ships no
//! headless compositor, so this is the "nested compositor" lane: the real client code, the real
//! protocol encoding, a compositor that logs every request it gets and sends the events a test
//! asks for. It checks that the title, app id, size limits and the server-side decoration request
//! all come before the first commit; that a configure maps the window with a buffer of the right
//! size (scale 1 and 1.5 through the viewport); that close reaches the app; that frames are paced
//! by callbacks; and the text-input-v3, keyboard, clipboard and drop flows.

use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wayland_protocols::wp::fractional_scale::v1::server::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::presentation_time::server::{wp_presentation, wp_presentation_feedback};
use wayland_protocols::wp::text_input::zv3::server::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};
use wayland_protocols::wp::viewporter::server::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::decoration::zv1::server::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::server::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_server::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_data_source, wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, Resource,
};
use wolkit::env::Environment;
use wolkit::event::{self, EventQueue, kind as ev};
use wolkit::scene::patch::{APPEND, Patch, Writer};
use wolkit::scene::{Kind, SceneRead};
use wolkit::window::native::{NativeWindow, Shared};
use wolkit::window::{Backend, Sink, WindowCore};

// ---- the compositor ----------------------------------------------------------------------

#[derive(Default)]
struct Objects {
    surface: Option<wl_surface::WlSurface>,
    xdg: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    text_input: Option<zwp_text_input_v3::ZwpTextInputV3>,
    device: Option<wl_data_device::WlDataDevice>,
    source: Option<wl_data_source::WlDataSource>,
    callbacks: Vec<wl_callback::WlCallback>,
    attached: Vec<wl_buffer::WlBuffer>,
    /// The offer we made and what it holds.
    offer: Option<(wl_data_offer::WlDataOffer, Vec<u8>)>,
}

/// Buffers by protocol id: width and height.
type Sizes = Arc<Mutex<Vec<(u32, (i32, i32))>>>;

struct Comp {
    log: Arc<Mutex<Vec<String>>>,
    o: Objects,
    sizes: Sizes,
    serial: u32,
    /// What our source sent when asked (mime, bytes).
    sent: Arc<Mutex<Vec<u8>>>,
}

impl Comp {
    fn log(&self, s: impl Into<String>) {
        self.log.lock().unwrap().push(s.into());
    }

    fn next(&mut self) -> u32 {
        self.serial += 1;
        self.serial
    }
}

struct Data;
impl wayland_server::backend::ClientData for Data {}

macro_rules! global {
    ($($iface:ty),* $(,)?) => {$(
        impl GlobalDispatch<$iface, ()> for Comp {
            fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, r: New<$iface>, _: &(), d: &mut DataInit<'_, Self>) {
                d.init(r, ());
            }
        }
    )*};
}

global!(
    wl_compositor::WlCompositor,
    wl_shm::WlShm,
    xdg_wm_base::XdgWmBase,
    zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_viewporter::WpViewporter,
    wp_presentation::WpPresentation,
    zwp_text_input_manager_v3::ZwpTextInputManagerV3,
    wl_data_device_manager::WlDataDeviceManager,
    wl_output::WlOutput,
);

impl GlobalDispatch<wl_seat::WlSeat, ()> for Comp {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<wl_seat::WlSeat>,
        _: &(),
        d: &mut DataInit<'_, Self>,
    ) {
        let seat = d.init(r, ());
        seat.capabilities(wl_seat::Capability::Keyboard | wl_seat::Capability::Pointer);
    }
}

/// Objects whose requests are only logged.
macro_rules! logged {
    ($($iface:ty => $name:literal),* $(,)?) => {$(
        impl Dispatch<$iface, ()> for Comp {
            fn request(s: &mut Self, _: &Client, _: &$iface, r: <$iface as Resource>::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
                s.log(format!("{}.{:?}", $name, r));
            }
        }
    )*};
}

logged!(
    wl_callback::WlCallback => "callback",
    wl_output::WlOutput => "output",
    wl_pointer::WlPointer => "pointer",
    wl_keyboard::WlKeyboard => "keyboard",
    wp_presentation_feedback::WpPresentationFeedback => "feedback",
    wp_fractional_scale_v1::WpFractionalScaleV1 => "fractional",
);

impl Dispatch<wl_buffer::WlBuffer, u32> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_buffer::WlBuffer,
        r: wl_buffer::Request,
        _: &u32,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        s.log(format!("buffer.{r:?}"));
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_compositor::WlCompositor,
        r: wl_compositor::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_compositor::Request::CreateSurface { id } => s.o.surface = Some(d.init(id, ())),
            wl_compositor::Request::CreateRegion { .. } => s.log("compositor.create_region"),
            _ => {}
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_surface::WlSurface,
        r: wl_surface::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        use wl_surface::Request as R;
        match r {
            R::Attach { buffer, .. } => {
                let size = buffer.as_ref().and_then(|b| {
                    let id = b.id().protocol_id();
                    s.sizes
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|x| x.0 == id)
                        .map(|x| x.1)
                });
                s.log(format!("surface.attach {size:?}"));
                if let Some(b) = buffer {
                    s.o.attached.push(b);
                }
            }
            R::Frame { callback } => {
                let c = d.init(callback, ());
                s.o.callbacks.push(c);
                s.log("surface.frame");
            }
            R::Commit => s.log("surface.commit"),
            R::SetBufferScale { scale } => s.log(format!("surface.buffer_scale {scale}")),
            R::DamageBuffer { width, height, .. } => {
                s.log(format!("surface.damage_buffer {width}x{height}"))
            }
            other => s.log(format!("surface.{other:?}")),
        }
    }
}

impl Dispatch<wl_shm::WlShm, ()> for Comp {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_shm::WlShm,
        r: wl_shm::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm::Request::CreatePool { id, .. } = r {
            d.init(id, ());
        }
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_shm_pool::WlShmPool,
        r: wl_shm_pool::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm_pool::Request::CreateBuffer {
            id, width, height, ..
        } = r
        {
            let b = d.init(id, 0u32);
            s.sizes
                .lock()
                .unwrap()
                .push((b.id().protocol_id(), (width, height)));
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &xdg_wm_base::XdgWmBase,
        r: xdg_wm_base::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let xdg_wm_base::Request::GetXdgSurface { id, .. } = r {
            s.o.xdg = Some(d.init(id, ()));
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &xdg_surface::XdgSurface,
        r: xdg_surface::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        match r {
            xdg_surface::Request::GetToplevel { id } => s.o.toplevel = Some(d.init(id, ())),
            xdg_surface::Request::AckConfigure { serial } => s.log(format!("xdg.ack {serial}")),
            other => s.log(format!("xdg.{other:?}")),
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &xdg_toplevel::XdgToplevel,
        r: xdg_toplevel::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use xdg_toplevel::Request as R;
        match r {
            R::SetTitle { title } => s.log(format!("toplevel.title {title}")),
            R::SetAppId { app_id } => s.log(format!("toplevel.app_id {app_id}")),
            R::SetMinSize { width, height } => s.log(format!("toplevel.min {width}x{height}")),
            R::SetMaxSize { width, height } => s.log(format!("toplevel.max {width}x{height}")),
            other => s.log(format!("toplevel.{other:?}")),
        }
    }
}

impl Dispatch<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, ()> for Comp {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
        r: zxdg_decoration_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id, .. } = r {
            d.init(id, ());
        }
    }
}

impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        deco: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        r: zxdg_toplevel_decoration_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let zxdg_toplevel_decoration_v1::Request::SetMode { mode } = r {
            s.log(format!("decoration.mode {mode:?}"));
            deco.configure(zxdg_toplevel_decoration_v1::Mode::ServerSide);
        }
    }
}

impl Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        r: wp_fractional_scale_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let wp_fractional_scale_manager_v1::Request::GetFractionalScale { id, .. } = r {
            s.o.fractional = Some(d.init(id, ()));
        }
    }
}

impl Dispatch<wp_viewporter::WpViewporter, ()> for Comp {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wp_viewporter::WpViewporter,
        r: wp_viewporter::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let wp_viewporter::Request::GetViewport { id, .. } = r {
            d.init(id, ());
        }
    }
}

impl Dispatch<wp_viewport::WpViewport, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wp_viewport::WpViewport,
        r: wp_viewport::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let wp_viewport::Request::SetDestination { width, height } = r {
            s.log(format!("viewport.destination {width}x{height}"));
        }
    }
}

impl Dispatch<wp_presentation::WpPresentation, ()> for Comp {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wp_presentation::WpPresentation,
        r: wp_presentation::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let wp_presentation::Request::Feedback { callback, .. } = r {
            d.init(callback, ());
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_seat::WlSeat,
        r: wl_seat::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_seat::Request::GetKeyboard { id } => {
                let k = d.init(id, ());
                k.repeat_info(30, 500);
                s.o.keyboard = Some(k);
            }
            wl_seat::Request::GetPointer { id } => {
                d.init(id, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_text_input_manager_v3::ZwpTextInputManagerV3, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &zwp_text_input_manager_v3::ZwpTextInputManagerV3,
        r: zwp_text_input_manager_v3::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        if let zwp_text_input_manager_v3::Request::GetTextInput { id, .. } = r {
            s.o.text_input = Some(d.init(id, ()));
        }
    }
}

impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &zwp_text_input_v3::ZwpTextInputV3,
        r: zwp_text_input_v3::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use zwp_text_input_v3::Request as R;
        match r {
            R::Enable => s.log("ti.enable"),
            R::Disable => s.log("ti.disable"),
            R::Commit => s.log("ti.commit"),
            R::SetSurroundingText {
                text,
                cursor,
                anchor,
            } => s.log(format!("ti.surrounding {text:?} {cursor} {anchor}")),
            R::SetCursorRectangle {
                x,
                y,
                width,
                height,
            } => s.log(format!("ti.cursor {x},{y} {width}x{height}")),
            R::SetContentType { .. } => s.log("ti.content_type"),
            other => s.log(format!("ti.{other:?}")),
        }
    }
}

impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_data_device_manager::WlDataDeviceManager,
        r: wl_data_device_manager::Request,
        _: &(),
        _: &DisplayHandle,
        d: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_data_device_manager::Request::CreateDataSource { id } => {
                s.o.source = Some(d.init(id, ()));
                s.log("data.create_source");
            }
            wl_data_device_manager::Request::GetDataDevice { id, .. } => {
                s.o.device = Some(d.init(id, ()))
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_data_source::WlDataSource,
        r: wl_data_source::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let wl_data_source::Request::Offer { mime_type } = r {
            s.log(format!("source.offer {mime_type}"));
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_data_device::WlDataDevice,
        r: wl_data_device::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let wl_data_device::Request::SetSelection { source, .. } = r {
            s.log(format!("device.set_selection {}", source.is_some()));
        }
    }
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for Comp {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_data_offer::WlDataOffer,
        r: wl_data_offer::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use std::io::Write;
        use wl_data_offer::Request as R;
        match r {
            R::Accept { mime_type, .. } => s.log(format!("offer.accept {mime_type:?}")),
            R::Receive { mime_type, fd } => {
                s.log(format!("offer.receive {mime_type}"));
                if let Some((_, bytes)) = &s.o.offer {
                    let mut f = std::fs::File::from(fd);
                    f.write_all(bytes).unwrap();
                }
            }
            R::Finish => s.log("offer.finish"),
            R::SetActions { .. } => s.log("offer.set_actions"),
            R::Destroy => s.log("offer.destroy"),
            _ => {}
        }
    }
}

/// What a test asks the compositor to do.
enum Cmd {
    Configure(i32, i32),
    Scale120(u32),
    Close,
    Frames,
    ReleaseAll,
    KeyboardEnter(OwnedFd, u32),
    Key(u32, bool),
    TextInputEnter,
    Preedit(String, i32, i32),
    Commit(String),
    Delete(u32, u32),
    Done(u32),
    /// Ask the client's source for its text in this type.
    AskSource(String),
    /// Offer text as the clipboard.
    OfferSelection(&'static str, Vec<u8>),
    /// Start a drag of this type and content over the surface at a point, then drop it.
    DragDrop(&'static str, Vec<u8>, f64, f64),
}

impl Comp {
    fn run(&mut self, c: Cmd, dh: &DisplayHandle) {
        match c {
            Cmd::Configure(w, h) => {
                let serial = self.next();
                if let (Some(t), Some(x)) = (&self.o.toplevel, &self.o.xdg) {
                    let activated = 4u32.to_ne_bytes().to_vec();
                    t.configure(w, h, activated);
                    x.configure(serial);
                }
            }
            Cmd::Scale120(n) => {
                if let Some(f) = &self.o.fractional {
                    f.preferred_scale(n);
                }
            }
            Cmd::Close => {
                if let Some(t) = &self.o.toplevel {
                    t.close();
                }
            }
            Cmd::Frames => {
                for c in self.o.callbacks.drain(..) {
                    c.done(1);
                }
            }
            Cmd::ReleaseAll => {
                for b in self.o.attached.drain(..) {
                    b.release();
                }
            }
            Cmd::KeyboardEnter(fd, size) => {
                let serial = self.next();
                if let (Some(k), Some(s)) = (&self.o.keyboard, &self.o.surface) {
                    k.keymap(wl_keyboard::KeymapFormat::XkbV1, fd.as_fd(), size);
                    k.enter(serial, s, Vec::new());
                    k.modifiers(serial, 0, 0, 0, 0);
                }
            }
            Cmd::Key(key, down) => {
                let serial = self.next();
                if let Some(k) = &self.o.keyboard {
                    let st = if down {
                        wl_keyboard::KeyState::Pressed
                    } else {
                        wl_keyboard::KeyState::Released
                    };
                    k.key(serial, 0, key, st);
                    // A compositor tracks modifier keys and sends their state (Control is mask 4
                    // in the us keymap).
                    if key == 29 {
                        k.modifiers(serial, if down { 4 } else { 0 }, 0, 0, 0);
                    }
                }
            }
            Cmd::TextInputEnter => {
                if let (Some(t), Some(s)) = (&self.o.text_input, &self.o.surface) {
                    t.enter(s);
                }
            }
            Cmd::Preedit(text, b, e) => {
                if let Some(t) = &self.o.text_input {
                    t.preedit_string(Some(text), b, e);
                }
            }
            Cmd::Commit(text) => {
                if let Some(t) = &self.o.text_input {
                    t.commit_string(Some(text));
                }
            }
            Cmd::Delete(b, a) => {
                if let Some(t) = &self.o.text_input {
                    t.delete_surrounding_text(b, a);
                }
            }
            Cmd::Done(serial) => {
                if let Some(t) = &self.o.text_input {
                    t.done(serial);
                }
            }
            Cmd::AskSource(mime) => {
                let (mine, theirs) = UnixStream::pair().unwrap();
                if let Some(src) = &self.o.source {
                    src.send(mime, OwnedFd::from(theirs).as_fd());
                }
                let sent = self.sent.clone();
                std::thread::Builder::new()
                    .name("test-read-source".into())
                    .spawn(move || {
                        use std::io::Read;
                        let mut b = Vec::new();
                        let mut s = mine;
                        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        let _ = s.read_to_end(&mut b);
                        *sent.lock().unwrap() = b;
                    })
                    .unwrap();
            }
            Cmd::OfferSelection(mime, bytes) => {
                let offer = self.make_offer(dh, mime);
                if let Some(d) = &self.o.device {
                    d.selection(Some(&offer));
                }
                self.o.offer = Some((offer, bytes));
            }
            Cmd::DragDrop(mime, bytes, x, y) => {
                let offer = self.make_offer(dh, mime);
                let serial = self.next();
                if let (Some(d), Some(s)) = (&self.o.device, &self.o.surface) {
                    d.enter(serial, s, x, y, Some(&offer));
                    d.motion(0, x, y);
                    d.drop();
                }
                self.o.offer = Some((offer, bytes));
            }
        }
    }

    fn make_offer(&mut self, dh: &DisplayHandle, mime: &str) -> wl_data_offer::WlDataOffer {
        let d = self.o.device.clone().unwrap();
        let client = d.client().unwrap();
        let offer = client
            .create_resource::<wl_data_offer::WlDataOffer, (), Comp>(dh, 3, ())
            .unwrap();
        d.data_offer(&offer);
        offer.offer(mime.to_owned());
        offer
    }
}

// ---- the rig -----------------------------------------------------------------------------

struct Rig {
    win: NativeWindow,
    core: WindowCore,
    events: EventQueue,
    tx: Sender<Cmd>,
    log: Arc<Mutex<Vec<String>>>,
    sent: Arc<Mutex<Vec<u8>>>,
    seq: u32,
    _home: std::path::PathBuf,
}

fn serve(
    sock: UnixStream,
    rx: Receiver<Cmd>,
    log: Arc<Mutex<Vec<String>>>,
    sent: Arc<Mutex<Vec<u8>>>,
) {
    let mut display: Display<Comp> = Display::new().unwrap();
    let dh = display.handle();
    dh.create_global::<Comp, wl_compositor::WlCompositor, ()>(6, ());
    dh.create_global::<Comp, wl_shm::WlShm, ()>(1, ());
    dh.create_global::<Comp, xdg_wm_base::XdgWmBase, ()>(6, ());
    dh.create_global::<Comp, zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, ()>(1, ());
    dh.create_global::<Comp, wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, ()>(1, ());
    dh.create_global::<Comp, wp_viewporter::WpViewporter, ()>(1, ());
    dh.create_global::<Comp, wp_presentation::WpPresentation, ()>(1, ());
    dh.create_global::<Comp, wl_seat::WlSeat, ()>(7, ());
    dh.create_global::<Comp, zwp_text_input_manager_v3::ZwpTextInputManagerV3, ()>(1, ());
    dh.create_global::<Comp, wl_data_device_manager::WlDataDeviceManager, ()>(3, ());
    dh.create_global::<Comp, wl_output::WlOutput, ()>(4, ());
    display
        .handle()
        .insert_client(sock, Arc::new(Data))
        .unwrap();
    let mut comp = Comp {
        log,
        o: Objects::default(),
        sizes: Arc::default(),
        serial: 0,
        sent,
    };
    loop {
        match rx.recv_timeout(Duration::from_millis(1)) {
            Ok(c) => comp.run(c, &dh),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
        if display.dispatch_clients(&mut comp).is_err() {
            return;
        }
        let _ = display.flush_clients();
    }
}

impl Rig {
    fn new(tag: &str, min: (u32, u32), max: (u32, u32)) -> Rig {
        let (client, server) = UnixStream::pair().unwrap();
        let (tx, rx) = channel();
        let log = Arc::new(Mutex::new(Vec::new()));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (l2, s2) = (log.clone(), sent.clone());
        std::thread::Builder::new()
            .name("test-compositor".into())
            .spawn(move || serve(server, rx, l2, s2))
            .unwrap();
        let home = std::env::temp_dir().join(format!("wolkit-native-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let shared = Shared::from_socket(client).unwrap();
        let win = NativeWindow::new(shared, "org.example.native", &home).unwrap();
        let mut core = WindowCore::new(1, "Native test", 320, 200, 0, Environment::default());
        core.min = min;
        core.max = max;
        Rig {
            win,
            core,
            events: EventQueue::new(),
            tx,
            log,
            sent,
            seq: 0,
            _home: home,
        }
    }

    fn send(&self, c: Cmd) {
        self.tx.send(c).unwrap();
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn pump(&mut self) {
        let mut s = Sink {
            events: &mut self.events,
            window: 1,
            seq: 0,
        };
        self.win.pump(&mut self.core, &mut s);
    }

    /// Pump until `done` holds (5 s at most).
    fn until(&mut self, what: &str, done: impl Fn(&Rig) -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            self.pump();
            if done(self) {
                return;
            }
            assert!(
                Instant::now() < end,
                "timed out waiting for {what}; log: {:#?}",
                self.log()
            );
            let mut ready = Vec::new();
            wolkit::loop_::wait(&self.win.fds(), 2, &mut ready);
        }
    }

    fn logged(&mut self, what: &str, line: &str) {
        let l = line.to_owned();
        self.until(what, move |r| r.log().iter().any(|x| x.starts_with(&l)));
    }

    fn show(&mut self) {
        let mut s = Sink {
            events: &mut self.events,
            window: 1,
            seq: 0,
        };
        self.win.show(&mut self.core, &mut s).unwrap();
    }

    fn apply(&mut self, f: impl FnOnce(&mut Writer)) {
        let mut p = Writer::new();
        f(&mut p);
        self.seq += 1;
        let bytes = p.finish(self.seq).to_vec();
        let c = &mut self.core;
        let patch = Patch::parse(&bytes, &mut c.scratch).unwrap();
        c.scene.try_apply(&patch, &mut c.journal).unwrap();
        let mut s = Sink {
            events: &mut self.events,
            window: 1,
            seq: self.seq,
        };
        self.win.committed(c, &mut s).unwrap();
        c.scene.commit(&mut c.journal);
    }

    fn events(&mut self) -> Vec<(u16, u32, Vec<u8>)> {
        let mut buf = vec![0u8; 1 << 20];
        let n = self.events.read(&mut buf);
        event::parse(&buf[..n])
            .into_iter()
            .map(|(h, p)| (h.kind, h.node, p.to_vec()))
            .collect()
    }

    /// Show, configure at `w` by `h` and wait for the mapping commit.
    fn mapped(&mut self, w: i32, h: i32) {
        self.show();
        self.logged("the first commit", "surface.commit");
        self.send(Cmd::Configure(w, h));
        self.logged("a buffer", "surface.attach Some");
    }
}

fn index(log: &[String], prefix: &str) -> usize {
    log.iter()
        .position(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("{prefix} not in {log:#?}"))
}

fn u32_at(p: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(p[at..at + 4].try_into().unwrap())
}

// ---- tests -------------------------------------------------------------------------------

#[test]
fn limits_title_and_decoration_come_before_the_first_commit() {
    let mut r = Rig::new("limits", (200, 100), (800, 600));
    r.show();
    r.logged("the first commit", "surface.commit");
    let log = r.log();
    let commit = index(&log, "surface.commit");
    for l in [
        "toplevel.title Native test",
        "toplevel.app_id org.example.native",
        "toplevel.min 200x100",
        "toplevel.max 800x600",
        "decoration.mode Value(ServerSide)",
    ] {
        assert!(
            index(&log, l) < commit,
            "{l} after the first commit: {log:#?}"
        );
    }
    assert!(
        !log[..commit]
            .iter()
            .any(|l| l.starts_with("surface.attach")),
        "no buffer before the first configure"
    );
}

#[test]
fn configure_maps_with_a_buffer_held_inside_the_limits() {
    let mut r = Rig::new("configure", (200, 100), (800, 600));
    r.mapped(640, 480);
    let log = r.log();
    assert!(
        index(&log, "xdg.ack 1") < index(&log, "surface.attach Some((640, 480))"),
        "{log:#?}"
    );
    assert_eq!((r.core.width, r.core.height), (640, 480));
    let resize = r.events().into_iter().find(|e| e.0 == ev::RESIZE).unwrap();
    assert_eq!((u32_at(&resize.2, 0), u32_at(&resize.2, 4)), (640, 480));
    r.send(Cmd::Frames);
    r.send(Cmd::Configure(100, 50));
    r.logged("the clamped buffer", "surface.attach Some((200, 100))");
    assert_eq!(
        (r.core.width, r.core.height),
        (200, 100),
        "a configure below the minimum is held at it"
    );
}

#[test]
fn fractional_scale_draws_device_pixels_through_the_viewport() {
    let mut r = Rig::new("scale", (0, 0), (0, 0));
    r.mapped(400, 300);
    r.send(Cmd::Frames);
    r.send(Cmd::ReleaseAll);
    r.send(Cmd::Scale120(180));
    r.logged("the 1.5x buffer", "surface.attach Some((600, 450))");
    let log = r.log();
    assert!(
        log.iter().any(|l| l == "viewport.destination 400x300"),
        "{log:#?}"
    );
    assert_eq!(r.core.scale, 1.5);
    let last = r.events().into_iter().rfind(|e| e.0 == ev::RESIZE).unwrap();
    assert_eq!(f32::from_bits(u32_at(&last.2, 8)), 1.5);
}

#[test]
fn close_from_the_compositor_reaches_the_app() {
    let mut r = Rig::new("close", (0, 0), (0, 0));
    r.mapped(300, 200);
    r.events();
    r.send(Cmd::Close);
    r.until("the close request", |r| !r.events.is_empty());
    // (Only the close is pending: the mapping's resize was read above.)
    assert!(
        r.events()
            .iter()
            .any(|e| e.0 == ev::CLOSE_REQUEST && e.1 == 0)
    );
}

#[test]
fn frames_wait_for_the_callback_and_idle_asks_for_none() {
    let mut r = Rig::new("pacing", (0, 0), (0, 0));
    r.mapped(300, 200);
    let frames = |r: &Rig| r.log().iter().filter(|l| *l == "surface.frame").count();
    assert_eq!(frames(&r), 1);
    for i in 0..5u32 {
        r.apply(|p| {
            p.create(10 + i, Kind::Box).insert(0, 10 + i, APPEND);
        });
    }
    r.pump();
    assert_eq!(
        frames(&r),
        1,
        "five changes while the frame is out wait for its callback"
    );
    r.send(Cmd::Frames);
    r.send(Cmd::ReleaseAll);
    r.until("the next frame", |r| {
        r.log().iter().filter(|l| *l == "surface.frame").count() == 2
    });
    r.send(Cmd::Frames);
    // The callback fires with nothing to draw: no new frame is asked for.
    let commits = |r: &Rig| r.log().iter().filter(|l| *l == "surface.commit").count();
    let before = commits(&r);
    r.until("the callback to be seen", |r| !r.win.pacer.is_waiting());
    for _ in 0..20 {
        r.pump();
    }
    assert_eq!(commits(&r), before, "an idle window commits nothing");
    assert_eq!(frames(&r), 2);
}

fn field(r: &mut Rig) {
    r.apply(|p| {
        p.create(1, Kind::TextInput).insert(0, 1, APPEND);
        p.bind_event(1, event::bit(ev::TEXT_CHANGED));
        p.focus(1);
    });
}

#[test]
fn text_input_v3_enables_on_focus_and_applies_batches_at_done() {
    let mut r = Rig::new("ti", (0, 0), (0, 0));
    r.mapped(300, 200);
    r.send(Cmd::TextInputEnter);
    r.until("the enter", |_| true);
    r.pump();
    field(&mut r);
    r.logged("enable", "ti.enable");
    r.logged("the first state commit", "ti.commit");
    let log = r.log();
    let en = index(&log, "ti.enable");
    assert!(
        en < index(&log, "ti.surrounding \"\" 0 0")
            && index(&log, "ti.surrounding") < index(&log, "ti.commit")
    );
    // A preedit: drawn, not written.
    r.send(Cmd::Preedit("ka".into(), 2, 2));
    r.send(Cmd::Done(1));
    r.until("the preedit", |r| {
        r.win.input.field.editor.preedit().is_some()
    });
    assert_eq!(r.core.scene.text(1), "");
    // Commit replaces it.
    r.send(Cmd::Preedit(String::new(), -1, -1));
    r.send(Cmd::Commit("か".into()));
    r.send(Cmd::Done(1));
    r.until("the commit", |r| r.core.scene.text(1) == "か");
    r.logged("the new surrounding text", "ti.surrounding \"か\" 3 3");
    assert!(
        r.events()
            .iter()
            .any(|e| e.0 == ev::TEXT_CHANGED && e.1 == 1)
    );
    // Delete surrounding plus commit, as an input method fixing a word.
    r.send(Cmd::Delete(3, 0));
    r.send(Cmd::Commit("漢".into()));
    r.send(Cmd::Done(2));
    r.until("the replacement", |r| r.core.scene.text(1) == "漢");
    // Leaving the field disables the input method.
    r.apply(|p| {
        p.focus(0);
    });
    r.logged("disable", "ti.disable");
}

fn keymap(dir: &std::path::Path, layout: &str, variant: &str) -> (OwnedFd, u32) {
    use xkbcommon::xkb;
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let km = xkb::Keymap::new_from_names(
        &ctx,
        "",
        "",
        layout,
        variant,
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .unwrap();
    let mut text = km.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1).into_bytes();
    text.push(0);
    let path = dir.join(format!("keymap-{layout}-{variant}"));
    std::fs::write(&path, &text).unwrap();
    let f = std::fs::File::open(&path).unwrap();
    (OwnedFd::from(f), u32::try_from(text.len()).unwrap())
}

#[test]
fn keys_through_the_keymap_type_into_the_focused_field() {
    let mut r = Rig::new("keys", (0, 0), (0, 0));
    r.mapped(300, 200);
    let (fd, size) = keymap(&r._home, "us", "");
    r.send(Cmd::KeyboardEnter(fd, size));
    field(&mut r);
    // evdev codes: 35 h, 18 e, 38 l, 24 o.
    for k in [35, 18, 38, 38, 24] {
        r.send(Cmd::Key(k, true));
        r.send(Cmd::Key(k, false));
    }
    r.until("typed text", |r| r.core.scene.text(1) == "hello");
    // Backspace (14).
    r.send(Cmd::Key(14, true));
    r.send(Cmd::Key(14, false));
    r.until("backspace", |r| r.core.scene.text(1) == "hell");
}

#[test]
fn copy_offers_text_and_serves_it_to_a_paste() {
    let mut r = Rig::new("copy", (0, 0), (0, 0));
    r.mapped(300, 200);
    let (fd, size) = keymap(&r._home, "us", "");
    r.send(Cmd::KeyboardEnter(fd, size));
    field(&mut r);
    r.apply(|p| {
        p.set_text(1, "copy me");
    });
    // Control (29) + a (30), then Control + c (46).
    for (k, d) in [
        (29, true),
        (30, true),
        (30, false),
        (46, true),
        (46, false),
        (29, false),
    ] {
        r.send(Cmd::Key(k, d));
    }
    r.logged("the selection", "device.set_selection true");
    let log = r.log();
    assert!(
        log.iter()
            .any(|l| l == "source.offer text/plain;charset=utf-8"),
        "{log:#?}"
    );
    r.send(Cmd::AskSource("text/plain;charset=utf-8".into()));
    let sent = r.sent.clone();
    r.until("the source to write", move |_| {
        !sent.lock().unwrap().is_empty()
    });
    assert_eq!(r.sent.lock().unwrap().as_slice(), b"copy me");
}

#[test]
fn paste_reads_another_clients_selection_without_blocking() {
    let mut r = Rig::new("paste", (0, 0), (0, 0));
    r.mapped(300, 200);
    let (fd, size) = keymap(&r._home, "us", "");
    r.send(Cmd::KeyboardEnter(fd, size));
    field(&mut r);
    r.send(Cmd::OfferSelection(
        "text/plain;charset=utf-8",
        b"from elsewhere".to_vec(),
    ));
    r.until("the offer", |_| true);
    for _ in 0..10 {
        r.pump();
    }
    // Control + v (47).
    for (k, d) in [(29, true), (47, true), (47, false), (29, false)] {
        r.send(Cmd::Key(k, d));
    }
    r.until("the pasted text", |r| {
        r.core.scene.text(1) == "from elsewhere"
    });
    assert!(
        r.log()
            .iter()
            .any(|l| l == "offer.receive text/plain;charset=utf-8")
    );
}

#[test]
fn drop_on_a_part_that_asked_delivers_type_and_descriptor() {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    let mut r = Rig::new("drop", (0, 0), (0, 0));
    r.mapped(300, 200);
    r.apply(|p| {
        p.bind_event(0, event::bit(ev::DROP));
    });
    r.events();
    r.send(Cmd::DragDrop(
        "text/uri-list",
        b"file:///srv/a.txt\n".to_vec(),
        50.0,
        40.0,
    ));
    r.until("the drop", |r| !r.events.is_empty());
    let drop = r
        .events()
        .into_iter()
        .find(|e| e.0 == ev::DROP)
        .expect("a DROP event");
    let fd = i32::try_from(u32_at(&drop.2, 0)).unwrap();
    assert_eq!(f32::from_le_bytes(drop.2[4..8].try_into().unwrap()), 50.0);
    let len = usize::try_from(u32_at(&drop.2, 12)).unwrap();
    assert_eq!(&drop.2[16..16 + len], b"text/uri-list");
    r.logged("the receive", "offer.receive text/uri-list");
    let log = r.log();
    assert!(
        index(&log, "offer.accept Some(\"text/uri-list\")")
            < index(&log, "offer.receive text/uri-list")
    );
    // SAFETY: the engine handed this descriptor to the app (us) in the DROP record; we own it now.
    let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut got = Vec::new();
    f.read_to_end(&mut got).unwrap();
    assert_eq!(got, b"file:///srv/a.txt\n");
}

#[test]
fn dead_keys_compose_through_the_locale_table() {
    use wolkit::window::native::seat::Keyboard;
    let mut k = Keyboard::new("en_US.UTF-8");
    assert!(
        k.set_keymap_names("us", "intl"),
        "the us(intl) keymap exists"
    );
    // evdev 40 is the apostrophe key: dead acute on us(intl); 18 is e.
    assert_eq!(k.key(40, true), None, "a dead key waits for the next one");
    let e = k.key(18, true).expect("the composed key");
    assert_eq!(e.text, "é");
    assert_eq!(e.keysym, 0xe9);
    let plain = k.key(30, true).expect("a plain key");
    assert_eq!(plain.text, "a");
}
