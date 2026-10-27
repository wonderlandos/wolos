//! Batteries widget: this computer's battery and connected devices that report one.
//! UPower (system bus) when it runs: event-driven, devices included. Without it: the built-in
//! battery only (Linux sysfs, FreeBSD `hw.acpi.battery.*`), refreshed on the widget schedule.
use std::collections::HashMap;
use zbus::zvariant::OwnedValue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Computer,
    Mouse,
    Keyboard,
    Headphones,
    Phone,
    Tablet,
    Gamepad,
    Other,
}

impl DeviceKind {
    /// UPower `Type` (only device types that carry a battery the user owns).
    pub fn from_upower(t: u32) -> Option<DeviceKind> {
        Some(match t {
            2 => DeviceKind::Computer,
            5 => DeviceKind::Mouse,
            6 => DeviceKind::Keyboard,
            8 => DeviceKind::Phone,
            10 => DeviceKind::Tablet,
            12 => DeviceKind::Gamepad,
            17 | 19 => DeviceKind::Headphones,
            7 | 9 | 13 | 14 | 18 | 22 | 26 | 28 => DeviceKind::Other,
            _ => return None,
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            DeviceKind::Computer => "computer-symbolic",
            DeviceKind::Mouse => "input-mouse-symbolic",
            DeviceKind::Keyboard => "input-keyboard-symbolic",
            DeviceKind::Headphones => "audio-headset-symbolic",
            DeviceKind::Phone => "phone-symbolic",
            DeviceKind::Tablet => "input-tablet-symbolic",
            DeviceKind::Gamepad => "input-gaming-symbolic",
            DeviceKind::Other => "battery-good-symbolic",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DeviceKind::Computer => "This Computer",
            DeviceKind::Mouse => "Mouse",
            DeviceKind::Keyboard => "Keyboard",
            DeviceKind::Headphones => "Headphones",
            DeviceKind::Phone => "Phone",
            DeviceKind::Tablet => "Tablet",
            DeviceKind::Gamepad => "Controller",
            DeviceKind::Other => "Device",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub kind: DeviceKind,
    pub percent: u8,
    pub charging: bool,
}

/// One UPower device's properties -> a device to show (present, has a percentage).
pub fn from_upower(
    t: u32,
    percent: f64,
    state: u32,
    present: bool,
    power_supply: bool,
    model: &str,
) -> Option<Device> {
    let kind = DeviceKind::from_upower(t)?;
    if !present || !(0.0..=100.0).contains(&percent) {
        return None;
    }
    // A battery that does not power the computer is a peripheral (UPS, wireless gear).
    let kind = if kind == DeviceKind::Computer && !power_supply {
        DeviceKind::Other
    } else {
        kind
    };
    let name = if kind == DeviceKind::Computer || model.trim().is_empty() {
        kind.name().to_string()
    } else {
        model.trim().to_string()
    };
    Some(Device {
        name,
        kind,
        percent: percent.round() as u8,
        // 1 charging, 4 fully charged, 5 pending charge.
        charging: matches!(state, 1 | 4 | 5),
    })
}

/// This computer first, then peripherals by name.
pub fn order(mut v: Vec<Device>) -> Vec<Device> {
    v.sort_by(|a, b| {
        (a.kind != DeviceKind::Computer, &a.name).cmp(&(b.kind != DeviceKind::Computer, &b.name))
    });
    v
}

/// FreeBSD `hw.acpi.battery.life` (percent, -1 none) and `.state` (bit 1 = charging).
pub fn freebsd_battery(life: Option<i64>, state: Option<i64>) -> Option<Device> {
    let p = life.filter(|l| (0..=100).contains(l))?;
    Some(Device {
        name: DeviceKind::Computer.name().into(),
        kind: DeviceKind::Computer,
        percent: p as u8,
        charging: state.is_some_and(|s| s & 2 != 0),
    })
}

/// Built-in battery without UPower.
pub fn builtin(sys: &dyn crate::system::System) -> Vec<Device> {
    if cfg!(target_os = "freebsd") {
        use wol_core::system::Host;
        let h = wol_core::system::PosixHost;
        return freebsd_battery(
            h.sysctl_i64("hw.acpi.battery.life"),
            h.sysctl_i64("hw.acpi.battery.state"),
        )
        .into_iter()
        .collect();
    }
    sys.battery()
        .map(|b| Device {
            name: DeviceKind::Computer.name().into(),
            kind: DeviceKind::Computer,
            percent: b.percent,
            charging: b.status != crate::status::BatteryStatus::Discharging,
        })
        .into_iter()
        .collect()
}

fn get<T: TryFrom<OwnedValue>>(m: &HashMap<String, OwnedValue>, k: &str) -> Option<T> {
    m.get(k)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| T::try_from(v).ok())
}

const UPOWER: &str = "org.freedesktop.UPower";

fn devices(conn: &zbus::blocking::Connection) -> Option<Vec<Device>> {
    let reply = conn
        .call_method(
            Some(UPOWER),
            "/org/freedesktop/UPower",
            Some(UPOWER),
            "EnumerateDevices",
            &(),
        )
        .ok()?;
    let paths: Vec<zbus::zvariant::OwnedObjectPath> = reply.body().deserialize().ok()?;
    let mut out = Vec::new();
    for p in paths {
        let Ok(props) = zbus::blocking::fdo::PropertiesProxy::builder(conn)
            .destination(UPOWER)
            .and_then(|b| b.path(p.clone()))
            .and_then(|b| b.build())
        else {
            continue;
        };
        let Ok(iface) = zbus::names::InterfaceName::try_from("org.freedesktop.UPower.Device")
        else {
            continue;
        };
        let Ok(m) = props.get_all(iface) else {
            continue;
        };
        if let Some(d) = from_upower(
            get(&m, "Type").unwrap_or(0),
            get(&m, "Percentage").unwrap_or(-1.0),
            get(&m, "State").unwrap_or(0),
            get(&m, "IsPresent").unwrap_or(false),
            get(&m, "PowerSupply").unwrap_or(false),
            &get::<String>(&m, "Model").unwrap_or_default(),
        ) {
            out.push(d);
        }
    }
    Some(order(out))
}

/// Sends UPower's devices now and after each change. Sends `None` once when UPower is not
/// reachable: the caller then reads the built-in battery itself.
pub fn watch(tx: async_channel::Sender<Option<Vec<Device>>>) {
    let _ = std::thread::Builder::new()
        .name("wol-upower".into())
        .spawn(move || {
            let conn = zbus::blocking::Connection::system().ok();
            let first = conn.as_ref().and_then(devices);
            let (Some(conn), Some(mut last)) = (conn, first) else {
                let _ = tx.send_blocking(None);
                return;
            };
            if tx.send_blocking(Some(last.clone())).is_err() {
                return;
            }
            if let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) {
                for r in [
                    "type='signal',sender='org.freedesktop.UPower',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'",
                    "type='signal',sender='org.freedesktop.UPower',interface='org.freedesktop.UPower'",
                ] {
                    if let Ok(rule) = zbus::MatchRule::try_from(r) {
                        let _ = dbus.add_match_rule(rule);
                    }
                }
            }
            for msg in zbus::blocking::MessageIterator::from(&conn) {
                let Ok(msg) = msg else { break };
                if msg.header().message_type() != zbus::message::Type::Signal {
                    continue;
                }
                let now = devices(&conn).unwrap_or_default();
                if now != last {
                    last = now.clone();
                    if tx.send_blocking(Some(now)).is_err() {
                        break;
                    }
                }
                if tx.is_closed() {
                    break;
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upower_devices_to_widget_rows() {
        type Row = (
            u32,
            f64,
            u32,
            bool,
            bool,
            &'static str,
            Option<(&'static str, DeviceKind, u8, bool)>,
        );
        let cases: &[Row] = &[
            (
                2,
                82.4,
                2,
                true,
                true,
                "L21M3PF0",
                Some(("This Computer", DeviceKind::Computer, 82, false)),
            ),
            (
                2,
                100.0,
                4,
                true,
                true,
                "",
                Some(("This Computer", DeviceKind::Computer, 100, true)),
            ),
            (
                5,
                41.0,
                2,
                true,
                false,
                "MX Master 3",
                Some(("MX Master 3", DeviceKind::Mouse, 41, false)),
            ),
            (
                17,
                60.0,
                1,
                true,
                false,
                "",
                Some(("Headphones", DeviceKind::Headphones, 60, true)),
            ),
            (2, 50.0, 2, false, true, "", None),
            (1, 0.0, 0, true, true, "AC", None),
            (5, -1.0, 0, true, false, "x", None),
            (
                2,
                30.0,
                2,
                true,
                false,
                "UPS",
                Some(("UPS", DeviceKind::Other, 30, false)),
            ),
        ];
        for (t, p, s, present, ps, model, want) in cases {
            let got = from_upower(*t, *p, *s, *present, *ps, model)
                .map(|d| (d.name.clone(), d.kind, d.percent, d.charging));
            let want = want.map(|(n, k, p, c)| (n.to_string(), k, p, c));
            assert_eq!(got, want, "type {t} {model}");
        }
    }

    #[test]
    fn computer_first_and_freebsd_fallback() {
        let d = |name: &str, kind| Device {
            name: name.into(),
            kind,
            percent: 1,
            charging: false,
        };
        let v = order(vec![
            d("Mouse", DeviceKind::Mouse),
            d("This Computer", DeviceKind::Computer),
            d("AirPods", DeviceKind::Headphones),
        ]);
        let names: Vec<&str> = v.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["This Computer", "AirPods", "Mouse"]);
        assert_eq!(freebsd_battery(Some(-1), Some(0)), None);
        assert_eq!(freebsd_battery(None, None), None);
        let b = freebsd_battery(Some(77), Some(2)).unwrap();
        assert_eq!((b.percent, b.charging), (77, true));
    }

    /// Manual: prints UPower's view of this machine.
    #[test]
    #[ignore]
    fn live_upower() {
        let conn = zbus::blocking::Connection::system().unwrap();
        println!("{:#?}", devices(&conn));
    }
}
