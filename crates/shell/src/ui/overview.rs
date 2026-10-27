//! Mission Control surface: blurred wallpaper, desktops strip on top, the current desktop's
//! windows scaled into a grid (live screencopy via grim; app icon until it arrives).
use crate::hypr::Client;
use crate::kit::{
    surface::{Keyboard, Spec},
    widgets as w,
};
use crate::overview::{self, Rect};
use gtk::{gdk, glib, prelude::*};

/// One window card: its client and an optional thumbnail.
pub struct Window<'a> {
    pub client: &'a Client,
    pub thumb: Option<gdk::Texture>,
}

/// Height reserved for the desktops strip; windows fill the rest.
const STRIP: f64 = (w::OVERVIEW_DESKTOP.1 + 2 * w::S6 + w::S4) as f64;

/// The whole overview at `size`. `on_pick` gets "focuswindow address:X" or "workspace N".
pub fn content(
    size: (i32, i32),
    desktops: &[i64],
    current: i64,
    windows: Vec<Window>,
    on_pick: impl Fn(String) + Clone + 'static,
) -> (gtk::Widget, Vec<(String, gtk::Picture)>) {
    let root = gtk::Overlay::new();
    root.add_css_class("overview");
    root.set_child(Some(&super::lock::backdrop()));
    let col = w::vbox(0);
    root.add_overlay(&col);

    let strip = w::hbox(w::S4);
    strip.add_css_class("overview-desktops");
    strip.set_halign(gtk::Align::Center);
    for &id in desktops {
        let d = w::vbox(w::S1);
        let tile = gtk::Overlay::new();
        tile.add_css_class("overview-desktop");
        if id == current {
            tile.add_css_class("current");
        }
        // The wallpaper is an overlay so its natural size never widens the tile.
        let sizer = w::vbox(0);
        sizer.set_size_request(w::OVERVIEW_DESKTOP.0, w::OVERVIEW_DESKTOP.1);
        tile.set_child(Some(&sizer));
        tile.add_overlay(&super::lock::backdrop());
        tile.set_overflow(gtk::Overflow::Hidden);
        let b = gtk::Button::builder().child(&tile).build();
        b.add_css_class("overview-desktop-button");
        let pick = on_pick.clone();
        b.connect_clicked(move |_| pick(format!("workspace {id}")));
        d.append(&b);
        let cap = w::centered(&format!("Desktop {id}"), "overview-caption");
        cap.set_halign(gtk::Align::Center);
        d.append(&cap);
        strip.append(&d);
    }
    col.append(&strip);

    let fixed = gtk::Fixed::new();
    fixed.set_vexpand(true);
    col.append(&fixed);
    let (sw, sh) = (size.0 as f64, size.1 as f64);
    let pad = w::S8 as f64 * 2.0;
    let area = Rect {
        x: pad,
        y: 0.0,
        w: sw - 2.0 * pad,
        h: sh - STRIP - pad - w::OVERVIEW_TITLE as f64 - w::S6 as f64,
    };
    let sizes: Vec<(i32, i32)> = windows
        .iter()
        .map(|c| (c.client.size[0], c.client.size[1]))
        .collect();
    let mut pics = Vec::new();
    for (win, r) in windows
        .into_iter()
        .zip(overview::grid(&sizes, area, w::S8 as f64))
    {
        let card = w::vbox(w::S2);
        let frame = gtk::Overlay::new();
        frame.add_css_class("overview-window");
        frame.set_overflow(gtk::Overflow::Hidden);
        frame.set_size_request(r.w as i32, r.h as i32);
        let pic = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Fill)
            .can_shrink(true)
            .build();
        if let Some(t) = &win.thumb {
            pic.set_paintable(Some(t));
        }
        frame.set_child(Some(&pic));
        let icon_name = crate::apps::icon_for(&win.client.class, crate::apps::PINS, &[]);
        let icon = w::app_icon(&icon_name, w::ICON_XL);
        icon.set_halign(gtk::Align::Center);
        icon.set_valign(gtk::Align::Center);
        frame.add_overlay(&icon);
        let b = gtk::Button::builder().child(&frame).build();
        b.add_css_class("overview-window-button");
        let (pick, addr) = (on_pick.clone(), win.client.address.clone());
        b.connect_clicked(move |_| pick(format!("focuswindow address:{addr}")));
        card.append(&b);
        let t = w::centered(
            &crate::clipboard::preview(&win.client.title),
            "overview-title",
        );
        t.add_css_class("may-ellipsize");
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_max_width_chars(w::TILE_CHARS * 2);
        t.set_halign(gtk::Align::Center);
        card.append(&t);
        card.set_size_request(r.w as i32, -1);
        fixed.put(&card, r.x, r.y);
        pics.push((win.client.address.clone(), pic));
    }
    (root.upcast(), pics)
}

/// The layer surface; rebuilt from Hyprland state on every show.
pub fn build(app: &gtk::Application) -> gtk::Window {
    let win = super::window(
        app,
        "panel",
        Spec::panel("wol-panel-overview", [true; 4], Keyboard::Exclusive),
    );
    win.add_css_class("overview-window-surface");
    let keys = gtk::EventControllerKey::new();
    let w2 = win.clone();
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gdk::Key::Escape {
            w2.set_visible(false);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    win.add_controller(keys);
    win.connect_show(|win| {
        let clients = crate::hypr::clients();
        let current = crate::hypr::request("j/activeworkspace")
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            .and_then(|v| v["id"].as_i64())
            .unwrap_or(1);
        let size = win
            .surface()
            .and_then(|s| WidgetExt::display(win).monitor_at_surface(&s))
            .map(|m| (m.geometry().width(), m.geometry().height()))
            .unwrap_or((win.width(), win.height()));
        let here: Vec<Window> = clients
            .iter()
            .filter(|c| c.workspace.id == current)
            .map(|client| Window {
                client,
                thumb: None,
            })
            .collect();
        let w2 = win.clone();
        let (child, pics) = content(
            size,
            &overview::desktops(&clients, current),
            current,
            here,
            move |d| {
                w2.set_visible(false);
                let _ = crate::hypr::request(&format!("dispatch {d}"));
            },
        );
        win.set_child(Some(&child));
        // Each window's own contents (covered or not) via hyprland_toplevel_export_v1.
        for (addr, pic) in pics {
            glib::spawn_future_local(async move {
                let out = gtk::gio::spawn_blocking(move || {
                    std::process::Command::new("wol-window-shot")
                        .arg(&addr)
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| o.stdout)
                })
                .await;
                let Ok(Some(bytes)) = out else { return };
                if let Some(t) = texture(&bytes) {
                    pic.set_paintable(Some(&t));
                    if let Some(icon) = pic.next_sibling() {
                        icon.set_visible(false);
                    }
                }
            });
        }
    });
    win.upcast()
}

/// `wol-window-shot` output -> texture.
fn texture(bytes: &[u8]) -> Option<gdk::MemoryTexture> {
    use crate::winshot::{Layout, decode, layout};
    let (f, px) = decode(bytes)?;
    let format = match layout(f.format)? {
        Layout::Bgra => gdk::MemoryFormat::B8g8r8a8,
        Layout::Bgrx => gdk::MemoryFormat::B8g8r8x8,
        Layout::Rgba => gdk::MemoryFormat::R8g8b8a8,
        Layout::Rgbx => gdk::MemoryFormat::R8g8b8x8,
    };
    Some(gdk::MemoryTexture::new(
        f.width as i32,
        f.height as i32,
        format,
        &glib::Bytes::from(&px[..(f.stride * f.height) as usize]),
        f.stride as usize,
    ))
}
