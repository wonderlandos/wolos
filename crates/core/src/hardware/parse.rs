//! Pure parsers for FreeBSD device tools: `pciconf -lv`, `devinfo -v`, `usbconfig`, `kldstat -v`.
use std::collections::HashSet;

/// One `pciconf -lv` device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PciDev {
    /// Driver unit (`iwlwifi0`) or `noneN` when no driver attached.
    pub unit: String,
    /// `pci0:0:20:3`.
    pub location: String,
    /// Six hex digits, no `0x`: `028000`.
    pub class: String,
    pub vendor: u16,
    pub device: u16,
    pub vendor_name: String,
    pub device_name: String,
}

impl PciDev {
    pub fn attached(&self) -> bool {
        !unit_of(&self.unit, "none")
    }
}

fn hex16(s: &str) -> Option<u16> {
    let h = s.trim().trim_start_matches("0x");
    // devinfo prints some ids 32-bit wide (`vendor=0x00001af4`).
    u32::from_str_radix(h, 16)
        .ok()
        .and_then(|v| u16::try_from(v).ok())
}

fn kv(s: &str) -> Vec<(String, String)> {
    s.split_whitespace()
        .filter_map(|t| t.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.trim_matches('"').to_owned()))
        .collect()
}

/// `name` is a unit of `driver`: `iwlwifi0` for `iwlwifi`, not `iwlwifi_x0`.
pub fn unit_of(name: &str, driver: &str) -> bool {
    name.strip_prefix(driver)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

pub fn pciconf(text: &str) -> Vec<PciDev> {
    let mut v: Vec<PciDev> = Vec::new();
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) {
            let Some((unit, rest)) = line.split_once('@') else {
                continue;
            };
            let (loc, fields) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let mut d = PciDev {
                unit: unit.trim().to_owned(),
                location: loc.trim_end_matches(':').to_owned(),
                ..Default::default()
            };
            for (k, val) in kv(fields) {
                match k.as_str() {
                    "class" => d.class = val.trim_start_matches("0x").to_owned(),
                    "vendor" => d.vendor = hex16(&val).unwrap_or(0),
                    "device" => d.device = hex16(&val).unwrap_or(0),
                    _ => {}
                }
            }
            v.push(d);
        } else if let (Some(d), Some((k, val))) = (v.last_mut(), line.split_once('=')) {
            let val = val.trim().trim_matches('\'').to_owned();
            match k.trim() {
                "vendor" => d.vendor_name = val,
                "device" => d.device_name = val,
                _ => {}
            }
        }
    }
    v
}

/// One `devinfo -v` node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    /// Unit name (`drmn0`) or `unknown` when nothing attached.
    pub name: String,
    pub parent: Option<usize>,
    /// `pnpinfo` and `at` key/value pairs.
    pub info: Vec<(String, String)>,
}

impl Node {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.info
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

pub fn devinfo(text: &str) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new(); // (indent, index)
    for line in text.lines() {
        let body = line.trim_start();
        if body.is_empty() {
            continue;
        }
        let indent = line.len() - body.len();
        while stack.last().is_some_and(|(i, _)| *i >= indent) {
            stack.pop();
        }
        let name = body
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned();
        let info = body
            .find(" pnpinfo ")
            .or_else(|| body.find(" at "))
            .map(|i| kv(&body[i..]))
            .unwrap_or_default();
        nodes.push(Node {
            name,
            parent: stack.last().map(|(_, i)| *i),
            info,
        });
        stack.push((indent, nodes.len() - 1));
    }
    nodes
}

/// Node of the PCI device at `location` (`dbsf=pci0:0:2:0`).
pub fn pci_node(nodes: &[Node], location: &str) -> Option<usize> {
    nodes.iter().position(|n| n.get("dbsf") == Some(location))
}

/// Unit names of the direct children of `idx`.
pub fn children(nodes: &[Node], idx: usize) -> impl Iterator<Item = &str> {
    nodes
        .iter()
        .filter(move |n| n.parent == Some(idx))
        .map(|n| n.name.as_str())
}

/// One `usbconfig dump_device_desc dump_curr_config_desc show_ifdrv` device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsbDev {
    /// `ugen0.2`.
    pub ugen: String,
    pub desc: String,
    pub vendor: u16,
    pub product: u16,
    /// Class/subclass/protocol, six hex digits: the device's, then each interface's.
    pub classes: Vec<String>,
    /// Interface drivers (`ubt0`).
    pub drivers: Vec<String>,
}

pub fn usbconfig(text: &str) -> Vec<UsbDev> {
    let mut v: Vec<UsbDev> = Vec::new();
    let mut class = [0u16; 3];
    for line in text.lines() {
        if line.starts_with("ugen") {
            let Some((id, rest)) = line.split_once(": ") else {
                continue;
            };
            if id.matches('.').count() == 1 {
                let desc = rest
                    .strip_prefix('<')
                    .and_then(|r| r.split_once('>'))
                    .map_or("", |(d, _)| d);
                v.push(UsbDev {
                    ugen: id.to_owned(),
                    desc: desc.to_owned(),
                    ..Default::default()
                });
            } else if let (Some(d), Some(drv)) = (v.last_mut(), rest.split_once(": ")) {
                // `ugen0.2.0: ubt0: <...>`
                if !drv.0.starts_with('<') && drv.0.ends_with(|c: char| c.is_ascii_digit()) {
                    d.drivers.push(drv.0.to_owned());
                }
            }
            continue;
        }
        let (Some(d), Some((k, val))) = (v.last_mut(), line.split_once(" = ")) else {
            continue;
        };
        let n = hex16(val.split_whitespace().next().unwrap_or_default()).unwrap_or(0);
        match k.trim() {
            "idVendor" => d.vendor = n,
            "idProduct" => d.product = n,
            "bDeviceClass" | "bInterfaceClass" => class[0] = n,
            "bDeviceSubClass" | "bInterfaceSubClass" => class[1] = n,
            "bDeviceProtocol" | "bInterfaceProtocol" => {
                class[2] = n;
                let c = format!("{:02x}{:02x}{:02x}", class[0], class[1], class[2]);
                if !d.classes.contains(&c) {
                    d.classes.push(c);
                }
            }
            _ => {}
        }
    }
    v
}

/// Loaded kld file names (no `.ko`) and module names (no `bus/` prefix) from `kldstat -v`.
pub fn kldstat(text: &str) -> HashSet<String> {
    let mut s = HashSet::new();
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| {
            t.get(i)
                .is_some_and(|x| x.bytes().all(|b| b.is_ascii_digit()))
        };
        if t.len() >= 5 && num(0) && num(1) && t[2].starts_with("0x") {
            s.insert(t[4].trim_end_matches(".ko").to_owned());
        } else if t.len() == 2 && num(0) {
            s.insert(t[1].rsplit('/').next().unwrap_or(t[1]).to_owned());
        }
    }
    s
}

/// Non-empty trimmed lines (`pkg query %n`).
pub fn lines(text: &str) -> HashSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// PCI vendor id for a `hw.model` string.
pub fn cpu_vendor(model: &str) -> Option<u16> {
    if model.contains("Intel") {
        Some(0x8086)
    } else if model.contains("AMD") {
        Some(0x1022)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;

    #[test]
    fn pciconf_fixture() {
        let v = pciconf(PCICONF_ADL);
        assert_eq!(v.len(), 9);
        let gpu = &v[1];
        assert_eq!(
            (
                gpu.unit.as_str(),
                gpu.location.as_str(),
                gpu.class.as_str(),
                gpu.vendor,
                gpu.device
            ),
            ("vgapci0", "pci0:0:2:0", "030000", 0x8086, 0x46a8)
        );
        assert_eq!(gpu.device_name, "Alder Lake-UP3 GT2 [Iris Xe Graphics]");
        assert_eq!(gpu.vendor_name, "Intel Corporation");
        let wifi = v.iter().find(|d| d.class == "028000").unwrap();
        assert_eq!((wifi.unit.as_str(), wifi.device), ("iwlwifi0", 0x51f0));
        assert!(wifi.attached());
        assert!(!v.iter().find(|d| d.unit == "none1").unwrap().attached());
        // The VM's own output (spaces instead of a tab, no vendor string).
        let q = pciconf(
            "vgapci0@pci0:0:1:0:     class=0x030000 rev=0x02 hdr=0x00 vendor=0x1234 device=0x1111 subvendor=0x1af4 subdevice=0x1100\n    class      = display\n",
        );
        assert_eq!(
            (q[0].vendor, q[0].device, q[0].vendor_name.as_str()),
            (0x1234, 0x1111, "")
        );
        assert!(pciconf("").is_empty() && pciconf("garbage\n  x = y\n").is_empty());
    }

    #[test]
    fn devinfo_tree_children_and_pnp() {
        let n = devinfo(DEVINFO_ADL);
        let gpu = pci_node(&n, "pci0:0:2:0").unwrap();
        assert_eq!(n[gpu].name, "vgapci0");
        assert_eq!(children(&n, gpu).collect::<Vec<_>>(), vec!["drmn0"]);
        let cpu = n.iter().position(|x| x.name == "cpu0").unwrap();
        assert_eq!(
            children(&n, cpu).collect::<Vec<_>>(),
            vec!["hwpstate_intel0", "cpufreq0"]
        );
        let tp = n.iter().find(|x| x.get("_CID") == Some("PNP0C50")).unwrap();
        assert_eq!(
            (tp.name.as_str(), tp.get("_HID")),
            ("unknown", Some("ELAN0678"))
        );
        let ubt = n.iter().find(|x| x.name == "ubt0").unwrap();
        assert_eq!(ubt.get("ugen"), Some("ugen0.3"));
        // `vendor=0x00001af4` (virtio) is 32-bit wide.
        let vt = devinfo("vtnet0 <VirtIO> pnpinfo vendor=0x00001af4 device=0x1000\n");
        assert_eq!(hex16(vt[0].get("vendor").unwrap()), Some(0x1af4));
    }

    #[test]
    fn usbconfig_devices_classes_and_drivers() {
        let v = usbconfig(USBCONFIG_LAPTOP);
        assert_eq!(v.len(), 3);
        assert_eq!((v[0].ugen.as_str(), v[0].vendor), ("ugen0.1", 0x8086));
        let bt = &v[1];
        assert_eq!(
            (bt.ugen.as_str(), bt.vendor, bt.product, bt.desc.as_str()),
            ("ugen0.3", 0x8087, 0x0026, "vendor 0x8087 product 0x0026")
        );
        assert_eq!(bt.classes, vec!["e00101"]);
        assert_eq!(bt.drivers, vec!["ubt0"]);
        let cam = &v[2];
        assert_eq!((cam.vendor, cam.product), (0x04f2, 0xb6dd));
        assert_eq!(cam.classes, vec!["ef0201", "0e0100", "0e0200"]);
        assert!(cam.drivers.is_empty());
        assert!(usbconfig("No device match or lack of permissions.\n").is_empty());
    }

    #[test]
    fn kldstat_files_and_modules() {
        let s = kldstat(KLDSTAT);
        for want in [
            "kernel",
            "snd_hda",
            "hwpstate_intel",
            "re",
            "igc",
            "if_iwlwifi",
            "i915kms",
            "zfs",
        ] {
            assert!(s.contains(want), "{want}");
        }
        assert!(!s.contains("Name") && !s.contains("Contains"));
    }

    #[test]
    fn small_helpers() {
        assert!(unit_of("iwlwifi0", "iwlwifi") && unit_of("hwpstate12", "hwpstate"));
        assert!(!unit_of("hwpstate_intel0", "hwpstate") && !unit_of("iwlwifi", "iwlwifi"));
        assert_eq!(
            lines("a\n\n b \n"),
            HashSet::from(["a".to_string(), "b".to_string()])
        );
        assert_eq!(
            cpu_vendor("12th Gen Intel(R) Core(TM) i5-1235U"),
            Some(0x8086)
        );
        assert_eq!(cpu_vendor("AMD Ryzen 7 7840U"), Some(0x1022));
        assert_eq!(cpu_vendor("QEMU Virtual CPU version 2.5+"), None);
    }
}
