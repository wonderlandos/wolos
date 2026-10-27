//! Control Center detail pages: Wi-Fi networks (join with password), Bluetooth devices, Sound
//! output, Focus modes. Rows rebuild only when their data changed.
use super::{SOUND_SETTINGS, Ui, WIFI_SETTINGS, launch};
use crate::copy;
use crate::focus::{self, Focus};
use crate::kit::{
    controls::{DetailPage, DeviceRow},
    widgets as w,
};
use crate::system::{Action, ActionError, BtDevice, Output, WifiNetwork};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Networks listed on the Wi-Fi page.
const MAX_NETWORKS: usize = 8;

pub(super) struct Pages {
    wifi: DetailPage,
    nets: RefCell<Option<Vec<WifiNetwork>>>,
    join: Join,
    bluetooth: DetailPage,
    devices: RefCell<Option<Vec<BtDevice>>>,
    sound: DetailPage,
    outputs: RefCell<Option<Vec<Output>>>,
    dnd: DeviceRow,
}

/// Join sheet: password for a secured network, or name and password for a hidden one.
#[derive(Clone)]
struct Join {
    sheet: gtk::Revealer,
    title: gtk::Label,
    name: gtk::Entry,
    password: gtk::PasswordEntry,
    /// The network being joined; None = Other Network (the name field).
    ssid: Rc<RefCell<Option<String>>>,
}

impl Join {
    fn open(&self, ssid: Option<&str>) {
        *self.ssid.borrow_mut() = ssid.map(String::from);
        self.title.set_label(&match ssid {
            Some(s) => format!("Join “{s}”"),
            None => "Join Other Network".into(),
        });
        self.name.set_text("");
        self.name.set_visible(ssid.is_none());
        self.password.set_text("");
        self.sheet.set_reveal_child(true);
        if ssid.is_none() {
            self.name.grab_focus();
        } else {
            self.password.grab_focus();
        }
    }
}

fn back(ui: &Rc<Ui>, p: &DetailPage) {
    let u = ui.clone();
    p.back.connect_clicked(move |_| u.show("main"));
}

fn link(p: &DetailPage, argv: &'static [&'static str]) {
    if let Some(f) = &p.footer {
        f.connect_clicked(move |b| launch(b, argv));
    }
}

/// Joins `ssid`, with progress and the outcome on the Wi-Fi page; a refused password reopens the
/// sheet.
fn join(ui: &Rc<Ui>, page: &DetailPage, j: &Join, ssid: String, password: Option<String>) {
    page.set_status(&copy::wifi_joining(&ssid), true);
    let (p, j) = (page.clone(), j.clone());
    let secured = password.is_some();
    ui.act_with(Action::WifiJoin(ssid.clone(), password), move |r| match r {
        Ok(()) => p.set_status("", false),
        Err(e) => {
            p.set_status(&copy::wifi_join(e, &ssid), false);
            if e == ActionError::WrongPassword && secured {
                j.open(Some(&ssid));
            }
        }
    });
}

fn join_sheet(ui: &Rc<Ui>, page: &DetailPage) -> Join {
    let title = w::label("", "title");
    let name = w::text_field("Network name");
    name.set_visible(false);
    let password = w::password_field("Password");
    let col = w::vbox(w::S2);
    col.append(&title);
    col.append(&name);
    col.append(&password);
    let row = w::hbox(w::S2);
    row.append(&w::spacer());
    let j = Join {
        sheet: w::sheet(&col),
        title,
        name,
        password,
        ssid: Default::default(),
    };
    {
        let r = j.sheet.clone();
        row.append(&w::button("Cancel", w::Kind::Default, move || {
            r.set_reveal_child(false)
        }));
    }
    let go = {
        let (u, p, j) = (ui.clone(), page.clone(), j.clone());
        Rc::new(move || {
            let ssid = j
                .ssid
                .borrow()
                .clone()
                .unwrap_or_else(|| j.name.text().trim().to_string());
            if ssid.is_empty() {
                j.name.grab_focus();
                return;
            }
            let pw = j.password.text().to_string();
            j.password.set_text("");
            j.sheet.set_reveal_child(false);
            join(&u, &p, &j, ssid, (!pw.is_empty()).then_some(pw));
        })
    };
    {
        let g = go.clone();
        j.name.connect_activate(move |_| g());
        let g = go.clone();
        j.password.connect_activate(move |_| g());
    }
    row.append(&w::button("Join", w::Kind::Primary, move || go()));
    col.append(&row);
    page.set_sheet(&j.sheet);
    j
}

impl Pages {
    pub fn build(ui: &Rc<Ui>) -> Self {
        let wifi = DetailPage::new("Wi-Fi", Some(false), "Networks", Some("Wi-Fi Settings…"));
        back(ui, &wifi);
        link(&wifi, WIFI_SETTINGS);
        if let Some(t) = &wifi.toggle {
            let u = ui.clone();
            t.connect_active_notify(move |t| u.act(Action::Wifi(t.is_active())));
        }
        let join = join_sheet(ui, &wifi);
        ui.pages.add_named(&wifi.root, Some("wifi"));

        let bluetooth = DetailPage::new("Bluetooth", Some(false), "Devices", None);
        back(ui, &bluetooth);
        if let Some(t) = &bluetooth.toggle {
            let u = ui.clone();
            t.connect_active_notify(move |t| u.act(Action::Bluetooth(t.is_active())));
        }
        ui.pages.add_named(&bluetooth.root, Some("bluetooth"));

        let sound = DetailPage::new("Sound", None, "Output", Some("Sound Settings…"));
        back(ui, &sound);
        link(&sound, SOUND_SETTINGS);
        ui.pages.add_named(&sound.root, Some("sound"));

        let focus_page = DetailPage::new("Focus", None, "", None);
        back(ui, &focus_page);
        let dnd = DeviceRow::new(
            "weather-clear-night-symbolic",
            "Do Not Disturb",
            "Off",
            false,
            None,
        );
        let hour = DeviceRow::new(
            "preferences-system-time-symbolic",
            "For 1 hour",
            "",
            false,
            None,
        );
        let morning = DeviceRow::new(
            "preferences-system-time-symbolic",
            "Until tomorrow morning",
            "",
            false,
            None,
        );
        let set = |row: &DeviceRow, pick: fn() -> Focus| {
            let u = ui.clone();
            row.button.connect_clicked(move |_| {
                let _ = focus::write(pick());
                if let Some(f) = u.resync.borrow().clone() {
                    f()
                }
            });
        };
        set(&dnd, || {
            if focus::read().active() {
                Focus::Off
            } else {
                Focus::On
            }
        });
        set(&hour, || Focus::Until(focus::unix_now() + focus::HOUR));
        set(&morning, || {
            let t = crate::kit::now();
            Focus::Until(focus::unix_now() + focus::secs_to_morning(t.hour, t.minute, t.second))
        });
        focus_page.set_rows(
            &[
                dnd.button.clone().upcast(),
                hour.button.clone().upcast(),
                morning.button.clone().upcast(),
            ],
            "",
        );
        ui.pages.add_named(&focus_page.root, Some("focus"));

        Pages {
            wifi,
            nets: Default::default(),
            join,
            bluetooth,
            devices: Default::default(),
            sound,
            outputs: Default::default(),
            dnd,
        }
    }

    /// Opens the Other Network sheet on the Wi-Fi page (`wifi:other`).
    pub fn other_network(&self) {
        self.join.open(None);
    }

    /// Called from `ControlCenter::sync` (handlers muted).
    pub fn sync(&self, ui: &Rc<Ui>, f: Focus, focus_label: &str) {
        let s = &*ui.sys;
        self.wifi.set_toggle(s.wifi_enabled().unwrap_or(false));
        let nets = s.wifi_networks();
        if self.nets.borrow().as_ref() != Some(&nets) {
            let mut rows: Vec<gtk::Widget> = nets
                .iter()
                .take(MAX_NETWORKS)
                .map(|n| self.network_row(ui, n))
                .collect();
            if s.wifi_enabled() == Some(true) {
                let other = DeviceRow::new("view-more-symbolic", "Other Network…", "", false, None);
                let j = self.join.clone();
                other.button.connect_clicked(move |_| j.open(None));
                rows.push(other.button.upcast());
            }
            self.wifi.set_rows(&rows, "No networks found");
            *self.nets.borrow_mut() = Some(nets);
        }

        let bt = s.bluetooth();
        self.bluetooth.set_toggle(bt.unwrap_or(false));
        if let Some(t) = &self.bluetooth.toggle {
            t.set_sensitive(bt.is_some());
        }
        let devices = s.bt_devices();
        if self.devices.borrow().as_ref() != Some(&devices) {
            let rows: Vec<gtk::Widget> = devices
                .iter()
                .map(|d| {
                    let r = DeviceRow::new(
                        "bluetooth-active-symbolic",
                        &d.name,
                        if d.connected { "Connected" } else { "" },
                        d.connected,
                        None,
                    );
                    let (u, addr, on) = (ui.clone(), d.addr.clone(), d.connected);
                    r.button
                        .connect_clicked(move |_| u.act(Action::BtConnect(addr.clone(), !on)));
                    r.button.upcast()
                })
                .collect();
            let empty = if bt.is_none() {
                "Bluetooth is unavailable"
            } else {
                "No paired devices"
            };
            self.bluetooth.set_rows(&rows, empty);
            *self.devices.borrow_mut() = Some(devices);
        }

        let outputs = s.output_devices();
        if self.outputs.borrow().as_ref() != Some(&outputs) {
            let rows: Vec<gtk::Widget> = outputs
                .iter()
                .map(|o| {
                    let icon = if o.name.to_lowercase().contains("headphone") {
                        "audio-headphones-symbolic"
                    } else {
                        "audio-speakers-symbolic"
                    };
                    let r = DeviceRow::new(
                        icon,
                        &o.name,
                        "",
                        o.default,
                        o.default.then_some("object-select-symbolic"),
                    );
                    let (u, id) = (ui.clone(), o.id.clone());
                    r.button
                        .connect_clicked(move |_| u.act(Action::Output(id.clone())));
                    r.button.upcast()
                })
                .collect();
            self.sound.set_rows(&rows, "No output devices");
            *self.outputs.borrow_mut() = Some(outputs);
        }

        self.dnd.set_active(f.active());
        self.dnd.set_subtitle(focus_label);
    }

    fn network_row(&self, ui: &Rc<Ui>, n: &WifiNetwork) -> gtk::Widget {
        let r = DeviceRow::new(
            crate::system::signal_icon(n.signal),
            &n.ssid,
            if n.connected { "Connected" } else { "" },
            n.connected,
            n.secure.then_some("system-lock-screen-symbolic"),
        );
        let (n, u, j, p) = (n.clone(), ui.clone(), self.join.clone(), self.wifi.clone());
        r.button.connect_clicked(move |_| {
            if n.secure && !n.connected {
                j.open(Some(&n.ssid));
            } else if !n.connected {
                join(&u, &p, &j, n.ssid.clone(), None);
            }
        });
        r.button.upcast()
    }
}
