// SPDX-License-Identifier: BSD-2-Clause
//! The Wayland side of one native window: the globals it binds and the dispatch handlers. The
//! handlers only record what arrived as [`Proto`] values (and answer pings); the backend applies
//! them to the window between dispatches (`mod.rs`), so every behaviour can be tested by
//! injecting `Proto` values without a compositor, and the wire is tested against an in-process
//! one (`tests/native_wayland.rs`).

use super::seat::Keyboard;
use std::sync::Mutex;
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_data_source, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_dmabuf_feedback_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols::wp::presentation_time::client::{wp_presentation, wp_presentation_feedback};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// What arrived from the compositor, in order.
#[derive(Debug)]
pub enum Proto {
    /// xdg_toplevel.configure then xdg_surface.configure.
    Configure(super::sizing::Configure, u32),
    Close,
    /// The scale the compositor prefers for our surface (fractional, or an integer one).
    FractionalScale(f32),
    BufferScale(i32),
    /// The surface entered or left an output (its integer scale matters without the others).
    OutputsChanged,
    /// Server-side decoration: true when the compositor draws the title bar.
    Decoration(bool),
    FrameDone,
    Presented {
        ns: u64,
        refresh_ns: u32,
    },
    Discarded,
    BufferReleased(u32),
    PointerEnter {
        serial: u32,
        x: f32,
        y: f32,
    },
    PointerLeave,
    PointerMotion {
        x: f32,
        y: f32,
    },
    /// Linux button code, pressed or released.
    PointerButton {
        serial: u32,
        button: u32,
        pressed: bool,
    },
    /// Logical pixels (vertical, horizontal); discrete wheel steps arrive already scaled.
    PointerAxis {
        dx: f32,
        dy: f32,
    },
    PointerFrame,
    KeyboardEnter {
        serial: u32,
    },
    KeyboardLeave,
    /// A decoded key: keysym, pressed, modifiers (`MOD_*`), text it types, whether it repeats.
    Key {
        serial: u32,
        keysym: u32,
        pressed: bool,
        mods: u32,
        text: String,
        repeats: bool,
    },
    Modifiers,
    RepeatInfo {
        rate: i32,
        delay: i32,
    },
    TextInputEnter,
    TextInputLeave,
    Preedit {
        text: String,
        begin: i32,
        end: i32,
    },
    CommitString(String),
    DeleteSurrounding {
        before: u32,
        after: u32,
    },
    TextInputDone(u32),
    /// The clipboard now holds this offer (its types), or nothing.
    Selection(Option<u32>),
    DragEnter {
        serial: u32,
        offer: Option<u32>,
        x: f32,
        y: f32,
    },
    DragMotion {
        x: f32,
        y: f32,
    },
    DragLeave,
    Drop,
    /// A paste of what we copied was asked for: write the text to the descriptor.
    SourceSend {
        mime: String,
        fd: std::os::fd::OwnedFd,
    },
    SourceCancelled,
    ActivationToken(String),
}

/// Types an offer said it has, collected as its `offer` events arrive.
#[derive(Default)]
pub struct OfferData {
    pub id: u32,
    pub types: Mutex<Vec<String>>,
}

/// What dmabuf feedback said (for the GPU path, WK-W5): the main device and the formats.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dmabuf {
    pub main_device: Option<u64>,
    /// (fourcc, modifier) pairs of the format table.
    pub formats: Vec<(u32, u64)>,
    pub done: bool,
}

pub struct Client {
    pub out: Vec<Proto>,
    pub shm_formats: Vec<u32>,
    pub keyboard: Keyboard,
    pub dmabuf: Dmabuf,
    /// Outputs and their integer scales.
    pub outputs: Vec<(wl_output::WlOutput, i32)>,
    /// Outputs our surface is on.
    pub entered: Vec<wayland_client::backend::ObjectId>,
    /// Offers alive, by protocol id.
    pub offers: Vec<wl_data_offer::WlDataOffer>,
    /// Discrete wheel steps in this pointer frame (high resolution, 120 per step).
    wheel: (i32, i32),
    /// Axis values in this pointer frame.
    axis: (f32, f32),
}

/// Logical pixels per wheel step.
pub const WHEEL_STEP: f32 = 48.0;

impl Client {
    pub fn new(locale: &str) -> Client {
        Client {
            out: Vec::new(),
            shm_formats: Vec::new(),
            keyboard: Keyboard::new(locale),
            dmabuf: Dmabuf::default(),
            outputs: Vec::new(),
            entered: Vec::new(),
            offers: Vec::new(),
            wheel: (0, 0),
            axis: (0.0, 0.0),
        }
    }

    /// The offer with this id, if it is alive.
    pub fn offer(&self, id: u32) -> Option<&wl_data_offer::WlDataOffer> {
        self.offers
            .iter()
            .find(|o| o.data::<OfferData>().is_some_and(|d| d.id == id))
    }

    pub fn offer_types(&self, id: u32) -> Vec<String> {
        self.offer(id)
            .and_then(|o| o.data::<OfferData>())
            .map(|d| d.types.lock().map(|t| t.clone()).unwrap_or_default())
            .unwrap_or_default()
    }

    /// Forget an offer (destroying it).
    pub fn drop_offer(&mut self, id: u32) {
        if let Some(at) = self
            .offers
            .iter()
            .position(|o| o.data::<OfferData>().is_some_and(|d| d.id == id))
        {
            let o = self.offers.remove(at);
            o.destroy();
        }
    }

    /// The largest integer scale of the outputs the surface is on.
    pub fn output_scale(&self) -> i32 {
        self.outputs
            .iter()
            .filter(|(o, _)| self.entered.contains(&o.id()))
            .map(|(_, s)| *s)
            .max()
            .unwrap_or(1)
            .max(1)
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Globals that come and go later (a new output) are not needed by a window.
    }
}

delegate_noop!(Client: ignore wl_compositor::WlCompositor);
delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Client: ignore wl_data_device_manager::WlDataDeviceManager);
delegate_noop!(Client: ignore zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);
delegate_noop!(Client: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(Client: ignore wp_viewporter::WpViewporter);
delegate_noop!(Client: ignore wp_viewport::WpViewport);
delegate_noop!(Client: ignore zwp_text_input_manager_v3::ZwpTextInputManagerV3);
delegate_noop!(Client: ignore wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
delegate_noop!(Client: ignore wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);
delegate_noop!(Client: ignore xdg_activation_v1::XdgActivationV1);
delegate_noop!(Client: ignore zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1);

impl Dispatch<wl_shm::WlShm, ()> for Client {
    fn event(
        s: &mut Self,
        _: &wl_shm::WlShm,
        e: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_shm::Event::Format { format } = e {
            s.shm_formats.push(u32::from(format));
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, u32> for Client {
    fn event(
        s: &mut Self,
        _: &wl_buffer::WlBuffer,
        e: wl_buffer::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = e {
            s.out.push(Proto::BufferReleased(*id));
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for Client {
    fn event(
        s: &mut Self,
        _: &wl_callback::WlCallback,
        e: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = e {
            s.out.push(Proto::FrameDone);
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for Client {
    fn event(
        s: &mut Self,
        _: &wl_surface::WlSurface,
        e: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_surface::Event::Enter { output } => {
                s.entered.push(output.id());
                s.out.push(Proto::OutputsChanged);
            }
            wl_surface::Event::Leave { output } => {
                s.entered.retain(|o| *o != output.id());
                s.out.push(Proto::OutputsChanged);
            }
            wl_surface::Event::PreferredBufferScale { factor } => {
                s.out.push(Proto::BufferScale(factor))
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Client {
    fn event(
        s: &mut Self,
        o: &wl_output::WlOutput,
        e: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Scale { factor } = e {
            match s.outputs.iter_mut().find(|(x, _)| x == o) {
                Some(slot) => slot.1 = factor,
                None => s.outputs.push((o.clone(), factor)),
            }
            s.out.push(Proto::OutputsChanged);
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
    fn event(
        _: &mut Self,
        base: &xdg_wm_base::XdgWmBase,
        e: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = e {
            base.pong(serial);
        }
    }
}

/// The toplevel configure waits here for the surface configure that completes it.
#[derive(Default)]
pub struct ToplevelData {
    pub pending: Mutex<Option<super::sizing::Configure>>,
}

impl Dispatch<xdg_surface::XdgSurface, std::sync::Arc<ToplevelData>> for Client {
    fn event(
        s: &mut Self,
        _: &xdg_surface::XdgSurface,
        e: xdg_surface::Event,
        d: &std::sync::Arc<ToplevelData>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = e {
            let c = d
                .pending
                .lock()
                .ok()
                .and_then(|mut p| p.take())
                .unwrap_or_default();
            s.out.push(Proto::Configure(c, serial));
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, std::sync::Arc<ToplevelData>> for Client {
    fn event(
        s: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        e: xdg_toplevel::Event,
        d: &std::sync::Arc<ToplevelData>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } => {
                if let Ok(mut p) = d.pending.lock() {
                    *p = Some(super::sizing::Configure::with_states(
                        width, height, &states,
                    ));
                }
            }
            xdg_toplevel::Event::Close => s.out.push(Proto::Close),
            _ => {}
        }
    }
}

impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, ()> for Client {
    fn event(
        s: &mut Self,
        _: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        e: zxdg_toplevel_decoration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_toplevel_decoration_v1::Event::Configure { mode } = e {
            let server = matches!(
                mode,
                WEnum::Value(zxdg_toplevel_decoration_v1::Mode::ServerSide)
            );
            s.out.push(Proto::Decoration(server));
        }
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for Client {
    fn event(
        s: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        e: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = e {
            // The protocol's unit is 1/120.
            let f = f64::from(scale) / 120.0;
            s.out.push(Proto::FractionalScale(f as f32));
        }
    }
}

impl Dispatch<wp_presentation::WpPresentation, ()> for Client {
    fn event(
        _: &mut Self,
        _: &wp_presentation::WpPresentation,
        _: wp_presentation::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The clock id: presentation times are only compared with each other here.
    }
}

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, ()> for Client {
    fn event(
        s: &mut Self,
        _: &wp_presentation_feedback::WpPresentationFeedback,
        e: wp_presentation_feedback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wp_presentation_feedback::Event::Presented {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
                refresh,
                ..
            } => {
                let secs = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
                let ns = secs
                    .saturating_mul(1_000_000_000)
                    .saturating_add(u64::from(tv_nsec));
                s.out.push(Proto::Presented {
                    ns,
                    refresh_ns: refresh,
                });
            }
            wp_presentation_feedback::Event::Discarded => s.out.push(Proto::Discarded),
            _ => {}
        }
    }
}

impl Dispatch<zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1, ()> for Client {
    fn event(
        s: &mut Self,
        _: &zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
        e: zwp_linux_dmabuf_feedback_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_linux_dmabuf_feedback_v1::Event as E;
        match e {
            E::FormatTable { fd, size } => s.dmabuf.formats = super::dmabuf::read_table(fd, size),
            E::MainDevice { device } => s.dmabuf.main_device = super::dmabuf::dev_t(&device),
            E::Done => s.dmabuf.done = true,
            _ => {}
        }
    }
}

/// The seat's capabilities, recorded by a dedicated user data so the backend can read them.
#[derive(Default)]
pub struct SeatCaps(pub Mutex<u32>);

impl Dispatch<wl_seat::WlSeat, std::sync::Arc<SeatCaps>> for Client {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        e: wl_seat::Event,
        caps: &std::sync::Arc<SeatCaps>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = e
            && let Ok(mut c) = caps.0.lock()
        {
            *c = u32::from(capabilities);
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Client {
    fn event(
        s: &mut Self,
        _: &wl_pointer::WlPointer,
        e: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wl_pointer::Event as E;
        let f = |v: f64| v as f32;
        match e {
            E::Enter {
                serial,
                surface_x,
                surface_y,
                ..
            } => s.out.push(Proto::PointerEnter {
                serial,
                x: f(surface_x),
                y: f(surface_y),
            }),
            E::Leave { .. } => s.out.push(Proto::PointerLeave),
            E::Motion {
                surface_x,
                surface_y,
                ..
            } => s.out.push(Proto::PointerMotion {
                x: f(surface_x),
                y: f(surface_y),
            }),
            E::Button {
                serial,
                button,
                state,
                ..
            } => s.out.push(Proto::PointerButton {
                serial,
                button,
                pressed: matches!(state, WEnum::Value(wl_pointer::ButtonState::Pressed)),
            }),
            E::Axis { axis, value, .. } => match axis {
                WEnum::Value(wl_pointer::Axis::VerticalScroll) => s.axis.1 += f(value),
                WEnum::Value(wl_pointer::Axis::HorizontalScroll) => s.axis.0 += f(value),
                _ => {}
            },
            E::AxisValue120 { axis, value120 } => match axis {
                WEnum::Value(wl_pointer::Axis::VerticalScroll) => s.wheel.1 += value120,
                WEnum::Value(wl_pointer::Axis::HorizontalScroll) => s.wheel.0 += value120,
                _ => {}
            },
            E::Frame => {
                // A wheel's steps win over its axis values (which some compositors scale).
                let step = |w: i32, a: f32| {
                    if w != 0 {
                        w as f32 / 120.0 * WHEEL_STEP
                    } else {
                        a
                    }
                };
                let (dx, dy) = (step(s.wheel.0, s.axis.0), step(s.wheel.1, s.axis.1));
                if dx != 0.0 || dy != 0.0 {
                    s.out.push(Proto::PointerAxis { dx, dy });
                }
                s.wheel = (0, 0);
                s.axis = (0.0, 0.0);
                s.out.push(Proto::PointerFrame);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        s: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        e: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wl_keyboard::Event as E;
        match e {
            E::Keymap { format, fd, size } => {
                if matches!(format, WEnum::Value(wl_keyboard::KeymapFormat::XkbV1)) {
                    s.keyboard.set_keymap(fd, size);
                }
            }
            E::Enter { serial, .. } => s.out.push(Proto::KeyboardEnter { serial }),
            E::Leave { .. } => {
                s.keyboard.reset_compose();
                s.out.push(Proto::KeyboardLeave);
            }
            E::Key {
                serial, key, state, ..
            } => {
                let pressed = matches!(state, WEnum::Value(wl_keyboard::KeyState::Pressed));
                if let Some(k) = s.keyboard.key(key, pressed) {
                    s.out.push(Proto::Key {
                        serial,
                        keysym: k.keysym,
                        pressed,
                        mods: k.mods,
                        text: k.text,
                        repeats: k.repeats,
                    });
                }
            }
            E::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                s.keyboard
                    .modifiers(mods_depressed, mods_latched, mods_locked, group);
                s.out.push(Proto::Modifiers);
            }
            E::RepeatInfo { rate, delay } => s.out.push(Proto::RepeatInfo { rate, delay }),
            _ => {}
        }
    }
}

impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, ()> for Client {
    fn event(
        s: &mut Self,
        _: &zwp_text_input_v3::ZwpTextInputV3,
        e: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_text_input_v3::Event as E;
        s.out.push(match e {
            E::Enter { .. } => Proto::TextInputEnter,
            E::Leave { .. } => Proto::TextInputLeave,
            E::PreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => Proto::Preedit {
                text: text.unwrap_or_default(),
                begin: cursor_begin,
                end: cursor_end,
            },
            E::CommitString { text } => Proto::CommitString(text.unwrap_or_default()),
            E::DeleteSurroundingText {
                before_length,
                after_length,
            } => Proto::DeleteSurrounding {
                before: before_length,
                after: after_length,
            },
            E::Done { serial } => Proto::TextInputDone(serial),
            _ => return,
        });
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for Client {
    fn event(
        s: &mut Self,
        _: &wl_data_device::WlDataDevice,
        e: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wl_data_device::Event as E;
        let id_of = |o: &wl_data_offer::WlDataOffer| o.data::<OfferData>().map_or(0, |d| d.id);
        match e {
            E::DataOffer { id } => {
                // A client that never clears offers would grow without bound: keep the newest.
                if s.offers.len() >= 8 {
                    let old = s.offers.remove(0);
                    old.destroy();
                }
                s.offers.push(id);
            }
            E::Selection { id } => {
                let sel = id.as_ref().map(id_of);
                // Offers that are neither the selection nor a drag in progress are done with.
                s.out.push(Proto::Selection(sel));
            }
            E::Enter {
                serial, x, y, id, ..
            } => s.out.push(Proto::DragEnter {
                serial,
                offer: id.as_ref().map(id_of),
                x: x as f32,
                y: y as f32,
            }),
            E::Motion { x, y, .. } => s.out.push(Proto::DragMotion {
                x: x as f32,
                y: y as f32,
            }),
            E::Leave => s.out.push(Proto::DragLeave),
            E::Drop => s.out.push(Proto::Drop),
            _ => {}
        }
    }

    wayland_client::event_created_child!(Client, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, OfferData {
            id: next_offer_id(),
            types: Mutex::new(Vec::new()),
        }),
    ]);
}

/// Offer ids are this process's own numbers (protocol ids are reused).
fn next_offer_id() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Dispatch<wl_data_offer::WlDataOffer, OfferData> for Client {
    fn event(
        _: &mut Self,
        _: &wl_data_offer::WlDataOffer,
        e: wl_data_offer::Event,
        d: &OfferData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = e
            && let Ok(mut t) = d.types.lock()
        {
            super::mime::keep(&mut t, &mime_type);
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for Client {
    fn event(
        s: &mut Self,
        _: &wl_data_source::WlDataSource,
        e: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_data_source::Event::Send { mime_type, fd } => s.out.push(Proto::SourceSend {
                mime: mime_type,
                fd,
            }),
            wl_data_source::Event::Cancelled => s.out.push(Proto::SourceCancelled),
            _ => {}
        }
    }
}

impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for Client {
    fn event(
        s: &mut Self,
        _: &xdg_activation_token_v1::XdgActivationTokenV1,
        e: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = e {
            s.out.push(Proto::ActivationToken(token));
        }
    }
}
