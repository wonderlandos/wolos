//! Wi-Fi: the radio, the network in use, known networks and the ones in range; secured networks ask
//! for their password in the Join sheet. Joining goes through wol-admind (wpa_supplicant.conf).
use super::*;
use crate::kit::controls::DeviceRow;
use crate::system::WifiNetwork;

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("Wi-Fi");
    let status = status_label();
    let ctx = &e.ctx;
    let wifi_on = ctx.sys.wifi_enabled();
    let lists = w::vbox(w::S6);
    let toggle = {
        let (ctx, st, lists) = (ctx.clone(), status.clone(), lists.clone());
        w::toggle(wifi_on.unwrap_or(false), move |on| {
            lists.set_visible(on);
            ctx.act_say(Action::Wifi(on), &st);
        })
    };
    toggle.set_sensitive(wifi_on.is_some());
    lists.set_visible(wifi_on.unwrap_or(false));
    let hint = if wifi_on.is_none() {
        "No Wi-Fi hardware was found."
    } else {
        ""
    };
    p.append(&w::section(
        "",
        &[w::list_row(
            "network-wireless-symbolic",
            "Wi-Fi",
            hint,
            Some(toggle.upcast_ref()),
        )
        .upcast()],
    ));
    let join = join_sheet(e, &status);
    let current = w::vbox(0);
    let known = w::vbox(0);
    let others = w::vbox(0);
    lists.append(&current);
    lists.append(&known);
    lists.append(&others);
    p.append(&lists);
    p.append(&status);
    let fill = {
        let (current, known, others, join, e) = (
            current.clone(),
            known.clone(),
            others.clone(),
            join.clone(),
            e.clone(),
        );
        Rc::new(move |nets: Vec<WifiNetwork>, saved: Vec<String>| {
            for b in [&current, &known, &others] {
                clear(b);
            }
            let row = |n: &WifiNetwork| -> gtk::Widget {
                let sub = if n.connected { "Connected" } else { "" };
                let lock = n.secure.then_some("network-wireless-encrypted-symbolic");
                let r = DeviceRow::new(
                    crate::system::signal_icon(n.signal),
                    &n.ssid,
                    sub,
                    n.connected,
                    lock,
                );
                let (n, join) = (n.clone(), join.clone());
                r.button.connect_clicked(move |_| {
                    if !n.connected {
                        join(n.ssid.clone(), n.secure);
                    }
                });
                r.button.upcast()
            };
            if let Some(n) = nets.iter().find(|n| n.connected) {
                let details = form::nav_row(
                    "network-wired-symbolic",
                    "Network Details",
                    "Address, router and DNS",
                );
                details.connect_clicked({
                    let e = e.clone();
                    move |_| e.open("Network")
                });
                current.append(&w::section("", &[row(n), details.upcast()]));
            }
            let mine: Vec<gtk::Widget> = nets
                .iter()
                .filter(|n| !n.connected && saved.contains(&n.ssid))
                .map(row)
                .collect();
            if !mine.is_empty() {
                known.append(&w::section("Known Networks", &mine));
            }
            let rest: Vec<gtk::Widget> = nets
                .iter()
                .filter(|n| !n.connected && !saved.contains(&n.ssid))
                .take(12)
                .map(row)
                .collect();
            if rest.is_empty() {
                others.append(&w::section(
                    "Other Networks",
                    &[w::list_row("", "No other networks in range", "", None).upcast()],
                ));
            } else {
                others.append(&w::section("Other Networks", &rest));
            }
            // Empty groups leave the layout (no doubled spacing).
            for b in [&current, &known, &others] {
                b.set_visible(b.first_child().is_some());
            }
        })
    };
    {
        let (fill, sys) = (fill.clone(), ctx.sys.clone());
        off_thread(
            {
                let m = e.machine();
                move || m.known_networks()
            },
            move |saved| {
                let saved = Rc::new(saved);
                fill(sys.wifi_networks(), (*saved).clone());
                if let Some(rx) = sys.watch() {
                    let last = RefCell::new(sys.wifi_networks());
                    glib::spawn_future_local(async move {
                        while rx.recv().await.is_ok() {
                            let n = sys.wifi_networks();
                            if *last.borrow() != n {
                                fill(n.clone(), (*saved).clone());
                                *last.borrow_mut() = n;
                            }
                        }
                    });
                }
            },
        );
    }
    p
}

/// The Join sheet: `join(ssid, secure)` asks for the password of secured networks, joins open ones
/// at once. Progress and errors show on `status`.
fn join_sheet(e: &Rc<Env>, status: &gtk::Label) -> Rc<dyn Fn(String, bool)> {
    let sheet = e.sheets.join.clone();
    let title = w::label("", "title");
    title.set_wrap(true);
    let pw = w::password_field("Password");
    let body = w::vbox(w::S3);
    body.append(&title);
    body.append(&pw);
    let buttons = w::hbox(w::S2);
    buttons.append(&w::spacer());
    let ssid: Rc<RefCell<String>> = Default::default();
    {
        let r = sheet.clone();
        buttons.append(&w::button("Cancel", w::Kind::Default, move || {
            r.set_reveal_child(false)
        }));
    }
    let join: Rc<dyn Fn(String, Option<String>)> = {
        let (ctx, st) = (e.ctx.clone(), status.clone());
        Rc::new(move |id: String, pass: Option<String>| {
            say(&st, &format!("Joining “{id}”…"));
            let st = st.clone();
            let id2 = id.clone();
            ctx.act(
                Action::WifiJoin(id, pass),
                Rc::new(move |r| match r {
                    Ok(()) => say(&st, &format!("Connected to “{id2}”.")),
                    Err(e) => say(&st, &crate::copy::wifi_join(e, &id2)),
                }),
            );
        })
    };
    {
        let (r, pwc, id, join) = (sheet.clone(), pw.clone(), ssid.clone(), join.clone());
        let go = Rc::new(move || {
            let pass = pwc.text().to_string();
            pwc.set_text("");
            r.set_reveal_child(false);
            join(id.borrow().clone(), Some(pass));
        });
        let g = go.clone();
        pw.connect_activate(move |_| g());
        buttons.append(&w::button("Join", w::Kind::Primary, move || go()));
    }
    body.append(&buttons);
    sheet_body(&sheet, &body);
    Rc::new(move |name: String, secure: bool| {
        if secure {
            *ssid.borrow_mut() = name.clone();
            title.set_label(&format!("Enter the password for “{name}”"));
            sheet.set_reveal_child(true);
            pw.grab_focus();
        } else {
            join(name, None);
        }
    })
}
