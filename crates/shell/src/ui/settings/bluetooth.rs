//! Bluetooth: the radio and paired devices (connect or disconnect with a click).
use super::*;
use crate::kit::controls::DeviceRow;
use wol_core::bluetooth::{BtKind, BtState};

pub(crate) fn kind_icon(k: BtKind) -> &'static str {
    match k {
        BtKind::Audio => "audio-headphones-symbolic",
        BtKind::Keyboard => "input-keyboard-symbolic",
        BtKind::Mouse => "input-mouse-symbolic",
        BtKind::Computer => "computer-symbolic",
        BtKind::Phone | BtKind::Other => "bluetooth-active-symbolic",
    }
}

fn devices(e: &Rc<Env>, into: &gtk::Box, st: BtState, status: &gtk::Label) {
    let paired = e.sys().bt_devices();
    if paired.is_empty() {
        into.append(&w::section(
            "My Devices",
            &[w::list_row(
                "",
                "No devices yet",
                "Put a device in pairing mode to connect it the first time.",
                None,
            )
            .upcast()],
        ));
        return;
    }
    let rows: Vec<gtk::Widget> = paired
        .iter()
        .map(|d| {
            let kind = st
                .devices
                .iter()
                .find(|x| x.addr.eq_ignore_ascii_case(&d.addr))
                .map_or(BtKind::Other, |x| x.kind);
            let r = DeviceRow::new(
                kind_icon(kind),
                &d.name,
                if d.connected {
                    "Connected"
                } else {
                    "Not connected"
                },
                d.connected,
                None,
            );
            let row = Rc::new(r);
            let on = Rc::new(Cell::new(d.connected));
            let (ctx, addr, st, r2, on2) = (
                e.ctx.clone(),
                d.addr.clone(),
                status.clone(),
                row.clone(),
                on.clone(),
            );
            row.button.connect_clicked(move |_| {
                let want = !on2.get();
                r2.set_subtitle(if want {
                    "Connecting…"
                } else {
                    "Disconnecting…"
                });
                let (st, r3, on3) = (st.clone(), r2.clone(), on2.clone());
                ctx.act(
                    Action::BtConnect(addr.clone(), want),
                    Rc::new(move |res| {
                        let now = if res.is_ok() { want } else { on3.get() };
                        on3.set(now);
                        r3.set_active(now);
                        r3.set_subtitle(if now { "Connected" } else { "Not connected" });
                        say(&st, res.err().map_or("", |e| e.message()));
                    }),
                );
            });
            row.button.clone().upcast()
        })
        .collect();
    into.append(&w::section("My Devices", &rows));
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("Bluetooth");
    let status = status_label();
    let on = e.sys().bluetooth();
    let body = w::vbox(w::S6);
    if on.is_none() {
        p.append(&w::empty_state(
            "bluetooth-disabled-symbolic",
            "No Bluetooth Hardware",
            "This computer has no Bluetooth adapter. Plug in a USB adapter to use wireless devices.",
            None,
        ));
        return p;
    }
    let list = w::vbox(w::S6);
    list.set_visible(on.unwrap_or(false));
    let toggle = {
        let (ctx, st, list) = (e.ctx.clone(), status.clone(), list.clone());
        w::toggle(on.unwrap_or(false), move |v| {
            list.set_visible(v);
            ctx.act_say(Action::Bluetooth(v), &st);
        })
    };
    body.append(&w::section(
        "",
        &[w::list_row(
            "bluetooth-active-symbolic",
            "Bluetooth",
            &format!("This computer appears as “{}”.", e.host.hostname),
            Some(toggle.upcast_ref()),
        )
        .upcast()],
    ));
    body.append(&list);
    p.append(&body);
    p.append(&status);
    let (e2, st) = (e.clone(), status.clone());
    let m = e.machine();
    later(
        &list,
        move || m.bluetooth(),
        move |into, s| devices(&e2, into, s, &st),
    );
    p
}
