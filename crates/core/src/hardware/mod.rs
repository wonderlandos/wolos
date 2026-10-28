//! Generic hardware: detect -> match the [`catalog`] -> driver + firmware + quirks -> status.
//! One path for boot (`wol hw boot`), `wol hw`, Settings and scripts. Reads go through [`Host`]
//! (sysfs, /proc, `lspci -mm`, `pacman -Qq`; 2 s timeout per command, never a shell); matching,
//! status and the boot plan are pure. See archicture/modules/hardware.md.
pub mod catalog;
#[cfg(test)]
mod fixtures;
pub mod online;
pub mod parse;

use crate::system::Host;
use shared::hardware::{Device, HardwareSnapshot, Kind, State};
use std::collections::HashSet;

/// Where a device was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    Pci,
    Usb,
    /// `/sys/bus/acpi/devices`, matched on `_HID`/`_CID` (modalias).
    Acpi,
    /// The processor (vendor from /proc/cpuinfo).
    Cpu,
    /// `/sys/class/backlight` devices.
    Backlight,
}

impl Bus {
    pub fn name(self) -> &'static str {
        match self {
            Bus::Pci => "pci",
            Bus::Usb => "usb",
            Bus::Acpi => "acpi",
            Bus::Cpu => "cpu",
            Bus::Backlight => "backlight",
        }
    }
}

/// One catalogue row. The most specific matching row wins: narrowest device range, then a
/// vendor over any vendor, then the longest class prefix; ties go to the earlier row.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub name: &'static str,
    pub kind: Kind,
    pub bus: Bus,
    pub vendor: Option<u16>,
    /// Inclusive device/product id ranges; empty = any.
    pub devices: &'static [(u16, u16)],
    /// Class prefix: PCI class hex (`0280`), USB class/sub/proto hex of the device or any
    /// interface (`e00101`), ACPI `_HID`/`_CID`. Empty = any.
    pub class: &'static str,
    /// sysfs driver names (the `driver` link) that count as "driver attached". Empty = not checked.
    pub drivers: &'static [&'static str],
    /// Kernel modules (`_` spelling) this device needs loaded.
    pub klds: &'static [&'static str],
    /// Arch firmware packages.
    pub firmware: &'static [&'static str],
    /// Arch driver/userland packages.
    pub packages: &'static [&'static str],
    /// modprobe.d lines (`options <module> k=v`), used once every package is installed.
    pub loader: &'static [&'static str],
    pub notes: &'static [&'static str],
    /// false: no driver in this release (status `unsupported`).
    pub supported: bool,
}

pub const BASE: Entry = Entry {
    name: "",
    kind: Kind::Other,
    bus: Bus::Pci,
    vendor: None,
    devices: &[],
    class: "",
    drivers: &[],
    klds: &[],
    firmware: &[],
    packages: &[],
    loader: &[],
    notes: &[],
    supported: true,
};

impl Entry {
    pub fn matches(
        &self,
        bus: Bus,
        vendor: Option<u16>,
        device: Option<u16>,
        classes: &[&str],
    ) -> bool {
        self.bus == bus
            && self.vendor.is_none_or(|v| Some(v) == vendor)
            && (self.devices.is_empty()
                || device
                    .is_some_and(|d| self.devices.iter().any(|(lo, hi)| (*lo..=*hi).contains(&d))))
            && (self.class.is_empty() || classes.iter().any(|c| c.starts_with(self.class)))
    }

    /// Lower = more specific: the matching device range's width, then vendor, then class length.
    fn rank(&self, device: Option<u16>) -> (u32, bool, std::cmp::Reverse<usize>) {
        let span = self
            .devices
            .iter()
            .filter(|(lo, hi)| device.is_some_and(|d| (*lo..=*hi).contains(&d)))
            .map(|(lo, hi)| u32::from(hi - lo) + 1)
            .min()
            .unwrap_or(0x1_0000);
        (
            span,
            self.vendor.is_none(),
            std::cmp::Reverse(self.class.len()),
        )
    }
}

/// Most specific catalogue row for a device.
pub fn lookup(
    bus: Bus,
    vendor: Option<u16>,
    device: Option<u16>,
    classes: &[&str],
) -> Option<&'static Entry> {
    catalog::CATALOG
        .iter()
        .filter(|e| e.matches(bus, vendor, device, classes))
        .min_by_key(|e| e.rank(device))
}

/// A kernel module the catalogue may load: the wol-admind `hw-load` allow-list.
pub fn is_catalogue_kld(k: &str) -> bool {
    catalog::CATALOG.iter().any(|e| e.klds.contains(&k))
}

/// What the host has. `None` = unknown (never reported as missing).
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    /// Loaded modules (`/proc/modules`) plus built-ins (`modules.builtin`).
    pub loaded: Option<HashSet<String>>,
    /// Modules this kernel has (`modules.dep` + `modules.builtin`).
    pub available: Option<HashSet<String>>,
    /// Installed packages (`pacman -Qq`).
    pub installed: Option<HashSet<String>>,
}

impl Ctx {
    fn is_loaded(&self, k: &str) -> Option<bool> {
        Some(self.loaded.as_ref()?.contains(&parse::module(k)))
    }
    fn on_disk(&self, k: &str) -> Option<bool> {
        Some(self.available.as_ref()?.contains(&parse::module(k)))
    }
    fn is_installed(&self, p: &str) -> bool {
        self.installed.as_ref().is_none_or(|i| i.contains(p))
    }
}

/// Fill catalogue data and status into `d`. `units`: driver names bound to the device.
pub fn resolve(entry: Option<&Entry>, mut d: Device, units: &[&str], ctx: &Ctx) -> Device {
    let Some(e) = entry else {
        // A device of an interesting class the catalogue does not know.
        if d.driver.is_none() {
            d.state = State::Unsupported;
            d.hint = "no driver attached for this device".into();
        }
        return d;
    };
    d.kind = e.kind;
    // The bus's own description names generic matches better ("Haswell-ULT Integrated Graphics"),
    // unless it is empty or a bare `vvvv:dddd` id, or the row names an exact device range.
    let bare = d.name.is_empty() || d.name == d.id;
    if !e.devices.is_empty() || !e.supported || bare {
        d.name = e.name.into();
    }
    let own = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    d.klds = own(e.klds);
    d.firmware = own(e.firmware);
    d.packages = own(e.packages);
    d.notes.extend(own(e.notes));
    if !e.drivers.is_empty() {
        d.driver = units
            .iter()
            .find(|u| {
                e.drivers
                    .iter()
                    .any(|p| parse::module(p) == parse::module(u))
            })
            .map(|u| (*u).to_owned());
    }
    if !e.supported {
        d.state = State::Unsupported;
        d.hint = e
            .notes
            .first()
            .copied()
            .unwrap_or("no driver in this release")
            .into();
        return d;
    }
    d.missing_packages = e
        .firmware
        .iter()
        .chain(e.packages)
        .filter(|p| !ctx.is_installed(p))
        .map(|p| (*p).to_owned())
        .collect();
    let mut unavailable = Vec::new();
    for k in e.klds {
        match (ctx.is_loaded(k), ctx.on_disk(k)) {
            (Some(true), _) => {}
            (_, Some(false)) => unavailable.push(*k),
            (Some(false), _) => d.missing_klds.push((*k).to_owned()),
            (None, _) => {}
        }
    }
    if d.missing_packages.is_empty() {
        d.loader = own(e.loader);
    }
    let missing_driver_pkgs: Vec<&str> = e
        .packages
        .iter()
        .copied()
        .filter(|p| !ctx.is_installed(p))
        .collect();
    let fix = |what: &str, v: &[String]| format!("{what} {}: wol hw fix", v.join(" "));
    (d.state, d.hint) = if !missing_driver_pkgs.is_empty() {
        let v = own(&missing_driver_pkgs);
        (State::NeedsDriver, format!("{} --yes", fix("install", &v)))
    } else if !unavailable.is_empty() {
        let note = format!("no {} module in this kernel", unavailable.join(" "));
        (State::Unsupported, note)
    } else if !d.missing_packages.is_empty() {
        (
            State::NeedsFirmware,
            format!("{} --yes", fix("install", &d.missing_packages)),
        )
    } else if !d.missing_klds.is_empty() {
        (State::NeedsDriver, fix("load", &d.missing_klds))
    } else if e.bus != Bus::Cpu && !e.drivers.is_empty() && d.driver.is_none() {
        (
            State::NeedsDriver,
            "driver loaded but not attached: see the kernel log".into(),
        )
    } else {
        (State::Ready, String::new())
    };
    d
}

/// Kind for a PCI class the catalogue does not know (`None` = not listed).
fn pci_kind(class: &str) -> Option<Kind> {
    [
        ("03", Kind::Gpu),
        ("0280", Kind::Wifi),
        ("02", Kind::Ethernet),
        ("0401", Kind::Audio),
        ("0403", Kind::Audio),
        ("0d11", Kind::Bluetooth),
        ("0805", Kind::CardReader),
    ]
    .iter()
    .find(|(p, _)| class.starts_with(p))
    .map(|(_, k)| *k)
}

const CPUFREQ_DRIVER: &str = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver";

fn trimmed(host: &dyn Host, path: &str) -> Option<String> {
    host.read_text(path)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Basename of a sysfs `driver` link.
fn driver_of(host: &dyn Host, dev: &str) -> Option<String> {
    host.read_link(&format!("{dev}/driver"))
        .and_then(|t| t.rsplit('/').next().map(String::from))
        .filter(|s| !s.is_empty())
}

fn base(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// The CPU: catalogue row and its cpufreq driver (`intel_pstate`, `amd-pstate-epp`, ...).
fn cpu(host: &dyn Host, ctx: &Ctx) -> Option<Device> {
    let (vendor_id, model) = parse::cpuinfo(&host.read_text("/proc/cpuinfo")?);
    let model = model?;
    let vendor_id = vendor_id.unwrap_or_default();
    let freq = trimmed(host, CPUFREQ_DRIVER);
    let units: Vec<&str> = freq.iter().map(String::as_str).collect();
    let d = Device {
        kind: Kind::Cpu,
        name: model.clone(),
        bus: Bus::Cpu.name().into(),
        id: vendor_id.clone(),
        location: "cpu0".into(),
        ..Default::default()
    };
    let entry = lookup(Bus::Cpu, parse::cpu_vendor(&vendor_id), None, &[]);
    let mut d = resolve(entry, d, &units, ctx);
    d.name = model;
    if freq.is_none() {
        d.notes
            .push("no cpufreq driver (virtual machine or firmware setting)".into());
    }
    Some(d)
}

/// What the host has: loaded and available modules, installed packages.
pub fn context(host: &dyn Host) -> Ctx {
    let release = trimmed(host, "/proc/sys/kernel/osrelease").unwrap_or_default();
    let dir = format!("/usr/lib/modules/{release}");
    let builtin = host
        .read_text(&format!("{dir}/modules.builtin"))
        .map(|t| parse::module_paths(&t))
        .unwrap_or_default();
    let loaded = host.read_text("/proc/modules").map(|t| {
        let mut l = parse::proc_modules(&t);
        l.extend(builtin.iter().cloned());
        l
    });
    let available = host.read_text(&format!("{dir}/modules.dep")).map(|t| {
        let mut a = parse::module_paths(&t);
        a.extend(builtin.iter().cloned());
        a
    });
    Ctx {
        loaded,
        available,
        installed: host.run(&["pacman", "-Qq"]).map(|t| parse::lines(&t)),
    }
}

fn pci_names(host: &dyn Host) -> Vec<(String, String)> {
    host.run(&["lspci", "-mm"])
        .map(|t| parse::lspci_names(&t))
        .unwrap_or_default()
}

fn pci(host: &dyn Host, ctx: &Ctx, names: &[(String, String)], out: &mut Vec<Device>) {
    for p in host.list_dir("/sys/bus/pci/devices") {
        let id = |f: &str| {
            host.read_text(&format!("{p}/{f}"))
                .and_then(|s| parse::hex16(&s))
        };
        let (Some(vendor), Some(device)) = (id("vendor"), id("device")) else {
            continue;
        };
        let Some(class) = host
            .read_text(&format!("{p}/class"))
            .and_then(|s| parse::pci_class(&s))
        else {
            continue;
        };
        let entry = lookup(Bus::Pci, Some(vendor), Some(device), &[&class]);
        let Some(kind) = entry.map(|e| e.kind).or_else(|| pci_kind(&class)) else {
            continue;
        };
        let addr = base(&p);
        let driver = driver_of(host, &p);
        let units: Vec<&str> = driver.iter().map(String::as_str).collect();
        let pid = format!("{vendor:04x}:{device:04x}");
        let d = Device {
            kind,
            name: names
                .iter()
                .find(|(s, _)| s == parse::lspci_slot(addr))
                .map_or_else(|| pid.clone(), |(_, n)| n.clone()),
            bus: Bus::Pci.name().into(),
            id: pid,
            location: addr.to_owned(),
            driver: driver.clone(),
            ..Default::default()
        };
        out.push(resolve(entry, d, &units, ctx));
    }
}

fn usb(host: &dyn Host, ctx: &Ctx, out: &mut Vec<Device>) {
    let all = host.list_dir("/sys/bus/usb/devices");
    let code = |p: &str, k: &str| {
        let f = |s: &str| trimmed(host, &format!("{p}/b{k}{s}")).unwrap_or_else(|| "00".into());
        format!("{}{}{}", f("Class"), f("SubClass"), f("Protocol")).to_ascii_lowercase()
    };
    for p in &all {
        let id = |f: &str| {
            host.read_text(&format!("{p}/{f}"))
                .and_then(|s| parse::hex16(&s))
        };
        let (Some(vendor), Some(product)) = (id("idVendor"), id("idProduct")) else {
            continue;
        };
        let prefix = format!("{p}:");
        let ifs: Vec<&String> = all.iter().filter(|i| i.starts_with(&prefix)).collect();
        let mut classes = vec![code(p, "Device")];
        classes.extend(ifs.iter().map(|i| code(i, "Interface")));
        let cl: Vec<&str> = classes.iter().map(String::as_str).collect();
        let Some(entry) = lookup(Bus::Usb, Some(vendor), Some(product), &cl) else {
            continue;
        };
        let drivers: Vec<String> = ifs.iter().filter_map(|i| driver_of(host, i)).collect();
        let units: Vec<&str> = drivers.iter().map(String::as_str).collect();
        let d = Device {
            name: trimmed(host, &format!("{p}/product")).unwrap_or_default(),
            bus: Bus::Usb.name().into(),
            id: format!("{vendor:04x}:{product:04x}"),
            location: base(p).to_owned(),
            ..Default::default()
        };
        out.push(resolve(Some(entry), d, &units, ctx));
    }
}

/// Snapshot of the hardware present now. Never fails: missing sources give fewer devices.
pub fn read(host: &dyn Host) -> HardwareSnapshot {
    // The two commands (package list, PCI names) are independent: run them side by side.
    let (ctx, names) = shared::par::join(|| context(host), || pci_names(host));
    let mut devices: Vec<Device> = Vec::new();
    devices.extend(cpu(host, &ctx));
    pci(host, &ctx, &names, &mut devices);
    usb(host, &ctx, &mut devices);
    for p in host.list_dir("/sys/bus/acpi/devices") {
        let classes = host
            .read_text(&format!("{p}/modalias"))
            .map(|m| parse::acpi_ids(&m))
            .unwrap_or_default();
        let cl: Vec<&str> = classes.iter().map(String::as_str).collect();
        let Some(entry) = (!cl.is_empty())
            .then(|| lookup(Bus::Acpi, None, None, &cl))
            .flatten()
        else {
            continue;
        };
        let d = Device {
            bus: Bus::Acpi.name().into(),
            id: classes.join(","),
            location: base(&p).to_owned(),
            ..Default::default()
        };
        devices.push(resolve(Some(entry), d, &[], &ctx));
    }
    for b in host.list_dir("/sys/class/backlight") {
        let d = Device {
            bus: Bus::Backlight.name().into(),
            id: base(&b).to_owned(),
            location: b.clone(),
            ..Default::default()
        };
        devices.push(resolve(
            lookup(Bus::Backlight, None, None, &[]),
            d,
            &[],
            &ctx,
        ));
    }
    devices.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.location.cmp(&b.location))
    });
    HardwareSnapshot { devices }
}

/// modprobe options for this machine's hardware; modprobe reads them whenever a module loads.
pub const MODPROBE_CONF: &str = "/etc/modprobe.d/wolos-hw.conf";
/// Modules systemd-modules-load loads every boot (catalogue modules udev may not autoload).
pub const MODULES_LOAD: &str = "/etc/modules-load.d/wolos-hw.conf";
/// First line of every file `wol hw boot` owns.
pub const HEADER: &str =
    "# Written by `wol hw boot` for this machine's hardware (every boot). Do not edit.\n";

/// What `wol hw boot` does: write modprobe options and the boot module list, then load modules
/// now. Files first: a module reads its options only when it loads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BootPlan {
    pub modprobe: Vec<String>,
    /// (path, full content); only files that change.
    pub writes: Vec<(String, String)>,
}

fn dedup(it: impl Iterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for s in it {
        if !v.contains(&s) {
            v.push(s);
        }
    }
    v
}

/// Pure: `current(path)` is the file's text now.
pub fn boot_plan(snap: &HardwareSnapshot, current: &dyn Fn(&str) -> Option<String>) -> BootPlan {
    let body = |lines: &[String]| {
        let mut s = HEADER.to_owned();
        for l in lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    };
    let options = dedup(snap.devices.iter().flat_map(|d| d.loader.iter().cloned()));
    let modules = dedup(
        snap.devices
            .iter()
            .filter(|d| d.state != State::Unsupported)
            .flat_map(|d| d.klds.iter().cloned()),
    );
    let mut writes = Vec::new();
    for (path, lines) in [(MODPROBE_CONF, options), (MODULES_LOAD, modules)] {
        let want = body(&lines);
        match current(path) {
            Some(cur) if cur == want => {}
            None if lines.is_empty() => {}
            _ => writes.push((path.to_owned(), want)),
        }
    }
    BootPlan {
        modprobe: fixes(snap).0,
        writes,
    }
}

/// What `wol hw fix` offers: modules to load now, packages to install.
pub fn fixes(snap: &HardwareSnapshot) -> (Vec<String>, Vec<String>) {
    (
        dedup(
            snap.devices
                .iter()
                .flat_map(|d| d.missing_klds.iter().cloned()),
        ),
        dedup(
            snap.devices
                .iter()
                .flat_map(|d| d.missing_packages.iter().cloned()),
        ),
    )
}

#[cfg(test)]
mod tests;
