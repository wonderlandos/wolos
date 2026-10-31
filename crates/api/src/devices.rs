// SPDX-License-Identifier: GPL-2.0-or-later
//! Devices for libwol (ADR 0020). wold opens a USB device for an app with a grant for that
//! vendor and product, and passes the descriptor: the app never gets a path. Keyboards, disks and
//! hubs stay with the system: raw access to one reads what the person types or every account's
//! files. Serial numbers and Bluetooth addresses stay here.
use serde_json::{Value, json};
use std::fs::File;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::Path;
use wol_core::system::Host;

pub const USB_SYSFS: &str = "/sys/bus/usb/devices";
pub const USB_NODES: &str = "/dev/bus/usb";

const CLASS_HID: u8 = 3;
const CLASS_STORAGE: u8 = 8;
const CLASS_HUB: u8 = 9;

#[derive(Debug, PartialEq, Eq)]
pub struct Usb {
    pub bus: u32,
    pub dev: u32,
    pub vendor: u32,
    pub product: u32,
    pub name: String,
    /// The device's class and its interfaces'.
    pub classes: Vec<u8>,
}

impl Usb {
    fn is_iphone(&self) -> bool {
        self.vendor == 0x05ac && (0x1290..=0x12af).contains(&self.product)
    }

    /// Can an app be given this device?
    fn may_open(&self) -> bool {
        !self
            .classes
            .iter()
            .any(|c| [CLASS_HID, CLASS_STORAGE, CLASS_HUB].contains(c))
    }

    fn label(&self) -> String {
        if self.name.is_empty() {
            format!("USB device {:04x}:{:04x}", self.vendor, self.product)
        } else {
            self.name.clone()
        }
    }
}

fn text(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name))
        .ok()
        .map(|t| t.trim().to_owned())
}

fn hex(dir: &Path, name: &str) -> Option<u32> {
    u32::from_str_radix(&text(dir, name)?, 16).ok()
}

fn one(dir: &Path) -> Option<Usb> {
    let mut classes = vec![u8::try_from(hex(dir, "bDeviceClass")?).ok()?];
    let name = dir.file_name()?.to_string_lossy().into_owned();
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let interface = e
            .file_name()
            .to_string_lossy()
            .starts_with(&format!("{name}:"));
        if interface && let Some(c) = hex(&e.path(), "bInterfaceClass") {
            classes.push(u8::try_from(c).ok()?);
        }
    }
    let mut label = [text(dir, "manufacturer"), text(dir, "product")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    label.truncate(label.floor_char_boundary(shared::portal::MAX_TITLE));
    Some(Usb {
        bus: text(dir, "busnum")?.parse().ok()?,
        dev: text(dir, "devnum")?.parse().ok()?,
        vendor: hex(dir, "idVendor")?,
        product: hex(dir, "idProduct")?,
        name: label,
        classes,
    })
}

/// The USB devices under `root` (sysfs), root hubs and all.
pub fn usb(root: &Path) -> Vec<Usb> {
    let mut v: Vec<Usb> = std::fs::read_dir(root)
        .map(|d| d.flatten().filter_map(|e| one(&e.path())).collect())
        .unwrap_or_default();
    v.sort_by_key(|u| (u.bus, u.dev));
    v
}

/// What an app sees: no serial number, no address.
pub fn list(root: &Path, h: &dyn Host) -> Value {
    let mut all: Vec<Value> = usb(root)
        .iter()
        .filter(|u| !u.classes.contains(&CLASS_HUB))
        .map(|u| {
            json!({
                "kind": if u.is_iphone() { "iphone" } else { "usb" },
                "vendor": u.vendor,
                "product": u.product,
                "name": u.label(),
                "can_open": u.may_open(),
            })
        })
        .collect();
    all.extend(wol_core::bluetooth::read(h).devices.iter().map(|d| {
        json!({"kind": "bluetooth", "name": d.name, "connected": d.connected, "can_open": false})
    }));
    json!({ "devices": all })
}

/// The device to open for (vendor, product): the first that matches and may be handed out.
fn pick(all: &[Usb], vendor: u32, product: u32) -> Result<&Usb, &'static str> {
    let mut found = all
        .iter()
        .filter(|u| u.vendor == vendor && u.product == product)
        .peekable();
    if found.peek().is_none() {
        return Err("That device is not connected.");
    }
    found
        .find(|u| u.may_open())
        .ok_or("Keyboards, disks and hubs cannot be handed to an app.")
}

/// Open the device for the caller; its name and descriptor.
pub fn open(
    root: &Path,
    nodes: &Path,
    vendor: u32,
    product: u32,
) -> Result<(String, File), &'static str> {
    let all = usb(root);
    let u = pick(&all, vendor, product)?;
    // Numbers from sysfs, never text from the app.
    let node = nodes.join(format!("{:03}/{:03}", u.bus, u.dev));
    let f = File::options()
        .read(true)
        .write(true)
        .custom_flags(shared::sys::OPEN_QUIET | libc::O_NOFOLLOW)
        .open(node)
        .map_err(|_| "That device could not be opened.")?;
    let is_device = f.metadata().is_ok_and(|m| m.file_type().is_char_device());
    if !is_device {
        return Err("That device could not be opened.");
    }
    Ok((u.label(), f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use wol_core::system::fake::FakeHost;

    fn tree(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wol-usb-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn device(root: &Path, name: &str, ids: (&str, &str), (bus, dev): (u32, u32), class: u32) {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        for (k, v) in [
            ("idVendor", ids.0.to_owned()),
            ("idProduct", ids.1.to_owned()),
            ("busnum", bus.to_string()),
            ("devnum", dev.to_string()),
            ("bDeviceClass", "00".to_owned()),
            ("serial", "SECRET".to_owned()),
        ] {
            std::fs::write(d.join(k), format!("{v}\n")).unwrap();
        }
        let i = d.join(format!("{name}:1.0"));
        std::fs::create_dir_all(&i).unwrap();
        std::fs::write(i.join("bInterfaceClass"), format!("{class:02x}\n")).unwrap();
    }

    #[test]
    fn apps_see_no_serial_and_no_hub() {
        let r = tree("list");
        device(&r, "1-1", ("046d", "c077"), (1, 2), CLASS_HID.into());
        device(&r, "1-2", ("05ac", "12a8"), (1, 3), 0xff);
        device(&r, "2-0", ("1d6b", "0002"), (2, 1), CLASS_HUB.into());
        std::fs::write(r.join("1-2/product"), "iPhone\n").unwrap();
        let v = list(&r, &FakeHost::new(true));
        let all = v["devices"].as_array().unwrap();
        assert_eq!(all.len(), 2, "{v}");
        assert_eq!(all[0]["can_open"], false, "a mouse");
        assert_eq!(all[1]["kind"], "iphone");
        assert_eq!(all[1]["name"], "iPhone");
        assert_eq!(all[1]["vendor"], 0x05ac);
        assert!(!v.to_string().contains("SECRET"));
        let _ = std::fs::remove_dir_all(r);
    }

    #[test]
    fn a_keyboard_a_disk_and_a_hub_are_never_opened() {
        let r = tree("refuse");
        let n = tree("nodes");
        for (name, class) in [
            ("1-1", CLASS_HID),
            ("1-2", CLASS_STORAGE),
            ("1-3", CLASS_HUB),
        ] {
            let ids = (format!("{class:04x}"), "0001".to_owned());
            device(
                &r,
                name,
                (&ids.0, &ids.1),
                (1, u32::from(class)),
                class.into(),
            );
            let got = open(&r, &n, class.into(), 1);
            assert_eq!(
                got.err(),
                Some("Keyboards, disks and hubs cannot be handed to an app.")
            );
        }
        assert_eq!(
            open(&r, &n, 9, 9).err(),
            Some("That device is not connected.")
        );
        let _ = std::fs::remove_dir_all(r);
        let _ = std::fs::remove_dir_all(n);
    }

    #[test]
    fn only_a_device_node_is_opened() {
        let r = tree("node");
        let n = tree("plain");
        device(&r, "1-1", ("1234", "5678"), (1, 2), 0xff);
        std::fs::create_dir_all(n.join("001")).unwrap();
        std::fs::write(n.join("001/002"), "plain file").unwrap();
        let got = open(&r, &n, 0x1234, 0x5678);
        assert_eq!(got.err(), Some("That device could not be opened."));
        std::fs::remove_file(n.join("001/002")).unwrap();
        std::os::unix::fs::symlink("/dev/null", n.join("001/002")).unwrap();
        let got = open(&r, &n, 0x1234, 0x5678);
        assert_eq!(
            got.err(),
            Some("That device could not be opened."),
            "a link"
        );
        let _ = std::fs::remove_dir_all(r);
        let _ = std::fs::remove_dir_all(n);
    }
}
