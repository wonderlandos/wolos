//! wol-window-shot <hyprland window address>: one frame of that window via
//! hyprland_toplevel_export_v1, written to stdout in `shell::winshot` format.
use std::io::{Read, Seek, Write};
use std::os::fd::AsFd;
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool},
};

#[allow(non_upper_case_globals, non_camel_case_types, dead_code, clippy::all)]
mod proto {
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/hyprland-toplevel-export-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("protocols/hyprland-toplevel-export-v1.xml");
}
use proto::hyprland_toplevel_export_frame_v1::{self as frame_v1, HyprlandToplevelExportFrameV1};
use proto::hyprland_toplevel_export_manager_v1::HyprlandToplevelExportManagerV1;

#[derive(Default)]
struct State {
    shm: Option<wl_shm::WlShm>,
    manager: Option<HyprlandToplevelExportManagerV1>,
    frame: Option<shell::winshot::Frame>,
    buffer_done: bool,
    ready: bool,
    failed: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        s: &mut Self,
        r: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = e
        {
            match interface.as_str() {
                "wl_shm" => s.shm = Some(r.bind(name, 1, qh, ())),
                "hyprland_toplevel_export_manager_v1" => s.manager = Some(r.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<HyprlandToplevelExportFrameV1, ()> for State {
    fn event(
        s: &mut Self,
        _: &HyprlandToplevelExportFrameV1,
        e: frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } if s.frame.is_none() => {
                s.frame = Some(shell::winshot::Frame {
                    width,
                    height,
                    stride,
                    format: format.into(),
                })
            }
            frame_v1::Event::BufferDone => s.buffer_done = true,
            frame_v1::Event::Ready { .. } => s.ready = true,
            frame_v1::Event::Failed => s.failed = true,
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore HyprlandToplevelExportManagerV1);

fn main() {
    if let Err(e) = run() {
        eprintln!("wol-window-shot: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let addr = std::env::args()
        .nth(1)
        .ok_or("usage: wol-window-shot <address>")?;
    let handle = shell::winshot::handle(&addr).ok_or("bad address")?;
    let conn = Connection::connect_to_env()?;
    let mut q = conn.new_event_queue();
    let qh = q.handle();
    conn.display().get_registry(&qh, ());
    let mut s = State::default();
    q.roundtrip(&mut s)?;
    let (shm, manager) = (
        s.shm.clone().ok_or("no wl_shm")?,
        s.manager
            .clone()
            .ok_or("no hyprland_toplevel_export_manager_v1")?,
    );
    let frame = manager.capture_toplevel(0, handle, &qh, ());
    while !(s.buffer_done || s.failed) {
        q.blocking_dispatch(&mut s)?;
    }
    let f = s.frame.ok_or("no shm buffer offered")?;
    let size = (f.stride * f.height) as usize;
    // Anonymous file in the runtime dir: the compositor writes the pixels into it.
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let path = std::path::Path::new(&dir).join(format!("wol-window-shot-{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    let _ = std::fs::remove_file(&path);
    file.set_len(size as u64)?;
    let pool = shm.create_pool(file.as_fd(), size as i32, &qh, ());
    let fmt = wl_shm::Format::try_from(f.format).map_err(|_| "unknown shm format")?;
    let buffer = pool.create_buffer(
        0,
        f.width as i32,
        f.height as i32,
        f.stride as i32,
        fmt,
        &qh,
        (),
    );
    frame.copy(&buffer, 1);
    while !(s.ready || s.failed) {
        q.blocking_dispatch(&mut s)?;
    }
    if s.failed {
        return Err("capture failed".into());
    }
    let mut px = Vec::with_capacity(size);
    file.rewind()?;
    file.read_to_end(&mut px)?;
    std::io::stdout().write_all(&shell::winshot::encode(f, &px))?;
    Ok(())
}
