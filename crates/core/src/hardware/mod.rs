//! Generic hardware: detect -> match the [`catalog`] -> driver + firmware + quirks -> status.
//! One path for boot (`wol hw boot`), `wol hw`, Settings and scripts. Reads go through [`Host`]
//! (5 commands, 2 s timeout each, never a shell); matching, status and the boot plan are pure.
//! See archicture/modules/hardware.md.
pub mod catalog;
#[cfg(test)]
mod fixtures;
pub mod online;
pub mod parse;

use crate::system::Host;
use shared::hardware::{Device, HardwareSnapshot, Kind, SNAPSHOT_PATH, State};
use std::collections::HashSet;

/// Where a device was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    Pci,
    Usb,
    /// `devinfo` ACPI nodes, matched on `_HID`/`_CID`.
    Acpi,
    /// The processor (vendor from `hw.model`).
    Cpu,
    /// backlight(4) devices.
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
    /// Unit name prefixes that count as "driver attached" (`iwlwifi` for `iwlwifi0`, or a child
    /// such as `drmn`). Empty = attachment not checked.
    pub drivers: &'static [&'static str],
    pub klds: &'static [&'static str],
    pub firmware: &'static [&'static str],
    /// Driver/userland packages.
    pub packages: &'static [&'static str],
    /// loader.conf lines, used once every package and file they name is present.
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

/// A module the catalogue may load: the wol-admind `hw-load` allow-list.
pub fn is_catalogue_kld(k: &str) -> bool {
    catalog::CATALOG.iter().any(|e| e.klds.contains(&k))
}

/// What the host has. `None` = unknown (never reported as missing).
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    /// Loaded kld files and modules (`kldstat -v`).
    pub loaded: Option<HashSet<String>>,
    /// Module files on disk (`/boot/kernel`, `/boot/modules`), without `.ko`.
    pub available: Option<HashSet<String>>,
    /// Installed packages.
    pub installed: Option<HashSet<String>>,
    /// Files under `/boot/firmware` and `/boot/modules` (full paths), for loader lines.
    pub files: Option<HashSet<String>>,
}

impl Ctx {
    fn is_loaded(&self, k: &str) -> Option<bool> {
        let l = self.loaded.as_ref()?;
        Some(l.contains(k) || k.strip_prefix("if_").is_some_and(|m| l.contains(m)))
    }
    fn on_disk(&self, k: &str) -> Option<bool> {
        Some(self.available.as_ref()?.contains(k))
    }
    fn is_installed(&self, p: &str) -> bool {
        self.installed.as_ref().is_none_or(|i| i.contains(p))
    }
    fn file_exists(&self, p: &str) -> bool {
        self.files.as_ref().is_none_or(|f| f.contains(p))
    }
}

/// Fill catalogue data and status into `d`. `units`: unit names attached to the device.
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
    // unless it is empty or a bare id, or the row explains an unsupported device.
    let bare = d.name.is_empty() || d.name.starts_with("vendor 0x");
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
            .find(|u| e.drivers.iter().any(|p| parse::unit_of(u, p)))
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
    let path_ok = |l: &&str| {
        l.split_once('=')
            .map(|(_, v)| v.trim_matches('"'))
            .is_none_or(|v| !v.starts_with('/') || ctx.file_exists(v))
    };
    if d.missing_packages.is_empty() && e.loader.iter().all(path_ok) {
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
        let h = if d.loader.is_empty() {
            "driver loaded but not attached: see dmesg"
        } else {
            "restart to load the driver (loader.conf)"
        };
        (State::NeedsDriver, h.into())
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

/// The CPU: catalogue row, cpufreq driver, and the powerd decision (powerd needs cpufreq(4); with
/// Speed Shift the hardware picks P-states and powerd has nothing to do).
fn cpu(host: &dyn Host, nodes: &[parse::Node], ctx: &Ctx) -> Option<Device> {
    let model = host.sysctl_string("hw.model")?;
    let vendor = parse::cpu_vendor(&model);
    let cpu0 = nodes.iter().position(|n| n.name == "cpu0");
    let units: Vec<&str> = cpu0
        .map(|i| parse::children(nodes, i).collect())
        .unwrap_or_default();
    let d = Device {
        kind: Kind::Cpu,
        name: model.trim().to_owned(),
        bus: Bus::Cpu.name().into(),
        id: match vendor {
            Some(0x8086) => "GenuineIntel",
            Some(0x1022) => "AuthenticAMD",
            _ => "",
        }
        .into(),
        location: "cpu0".into(),
        ..Default::default()
    };
    let mut d = resolve(lookup(Bus::Cpu, vendor, None, &[]), d, &units, ctx);
    d.name = model.trim().to_owned();
    let cpufreq = host.sysctl_i64("dev.cpu.0.freq").is_some();
    let hwp = d
        .driver
        .as_deref()
        .is_some_and(|u| parse::unit_of(u, "hwpstate_intel"));
    let powerd = cpufreq && !hwp;
    d.rc.push(format!(
        "powerd_enable=\"{}\"",
        if powerd { "YES" } else { "NO" }
    ));
    if !cpufreq {
        d.notes
            .push("no cpufreq(4) driver (virtual machine or firmware setting): powerd off".into());
    } else if hwp {
        d.notes
            .push("Speed Shift: the processor picks its own speed; powerd off".into());
    }
    Some(d)
}

/// Snapshot of the hardware present now. Never fails: missing tools give fewer devices.
pub fn read(host: &dyn Host) -> HardwareSnapshot {
    if !host.is_freebsd() {
        return HardwareSnapshot::default();
    }
    let run = |argv: &[&str]| host.run(argv);
    let pci = run(&["pciconf", "-lv"])
        .map(|t| parse::pciconf(&t))
        .unwrap_or_default();
    let nodes = run(&["devinfo", "-v"])
        .map(|t| parse::devinfo(&t))
        .unwrap_or_default();
    let usb = run(&[
        "usbconfig",
        "dump_device_desc",
        "dump_curr_config_desc",
        "show_ifdrv",
    ])
    .map(|t| parse::usbconfig(&t))
    .unwrap_or_default();
    let stems = |dirs: &[&str]| -> Option<HashSet<String>> {
        let s: HashSet<String> = dirs
            .iter()
            .flat_map(|d| host.list_dir(d))
            .filter_map(|p| p.rsplit('/').next()?.strip_suffix(".ko").map(String::from))
            .collect();
        (!s.is_empty()).then_some(s)
    };
    let files: HashSet<String> = ["/boot/firmware", "/boot/modules"]
        .iter()
        .flat_map(|d| host.list_dir(d))
        .collect();
    let ctx = Ctx {
        loaded: run(&["kldstat", "-v"]).map(|t| parse::kldstat(&t)),
        available: stems(&["/boot/kernel", "/boot/modules"]),
        installed: run(&["pkg", "query", "%n"]).map(|t| parse::lines(&t)),
        files: (!files.is_empty()).then_some(files),
    };
    let mut devices: Vec<Device> = Vec::new();
    devices.extend(cpu(host, &nodes, &ctx));
    for p in &pci {
        let entry = lookup(Bus::Pci, Some(p.vendor), Some(p.device), &[&p.class]);
        let Some(kind) = entry.map(|e| e.kind).or_else(|| pci_kind(&p.class)) else {
            continue;
        };
        let mut units: Vec<&str> = Vec::new();
        if p.attached() {
            units.push(&p.unit);
        }
        if let Some(i) = parse::pci_node(&nodes, &p.location) {
            units.extend(parse::children(&nodes, i));
        }
        let d = Device {
            kind,
            name: if p.device_name.is_empty() {
                format!("{:04x}:{:04x}", p.vendor, p.device)
            } else {
                p.device_name.clone()
            },
            bus: Bus::Pci.name().into(),
            id: format!("{:04x}:{:04x}", p.vendor, p.device),
            location: p.location.clone(),
            driver: p.attached().then(|| p.unit.clone()),
            ..Default::default()
        };
        devices.push(resolve(entry, d, &units, &ctx));
    }
    for u in &usb {
        let classes: Vec<&str> = u.classes.iter().map(String::as_str).collect();
        let Some(entry) = lookup(Bus::Usb, Some(u.vendor), Some(u.product), &classes) else {
            continue;
        };
        let mut units: Vec<&str> = u.drivers.iter().map(String::as_str).collect();
        units.extend(
            nodes
                .iter()
                .filter(|n| n.get("ugen") == Some(&u.ugen))
                .map(|n| n.name.as_str()),
        );
        let d = Device {
            name: u.desc.clone(),
            bus: Bus::Usb.name().into(),
            id: format!("{:04x}:{:04x}", u.vendor, u.product),
            location: u.ugen.clone(),
            ..Default::default()
        };
        devices.push(resolve(Some(entry), d, &units, &ctx));
    }
    if usb.is_empty() && !host.is_root() {
        // usbconfig needs root: reuse what the boot snapshot saw.
        let boot: HardwareSnapshot = host
            .read_text(SNAPSHOT_PATH)
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        devices.extend(boot.devices.into_iter().filter(|d| d.bus == "usb"));
    }
    for n in &nodes {
        let classes: Vec<&str> = ["_HID", "_CID"].iter().filter_map(|k| n.get(k)).collect();
        if classes.is_empty() {
            continue;
        }
        if let Some(entry) = lookup(Bus::Acpi, None, None, &classes) {
            let d = Device {
                bus: Bus::Acpi.name().into(),
                id: classes.join(","),
                location: n.get("handle").unwrap_or_default().to_owned(),
                driver: (n.name != "unknown").then(|| n.name.clone()),
                ..Default::default()
            };
            devices.push(resolve(Some(entry), d, &[], &ctx));
        }
    }
    for b in host.list_dir("/dev/backlight") {
        let d = Device {
            bus: Bus::Backlight.name().into(),
            id: b.rsplit('/').next().unwrap_or_default().to_owned(),
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

/// Next-boot loader tunables for this machine's hardware.
pub const LOADER_CONF: &str = "/boot/loader.conf.d/wolos-hw.conf";
/// rc.conf.d files (`/etc/rc.conf.d/<service>`) override rc.conf per service.
pub const RC_CONF_DIR: &str = "/etc/rc.conf.d";
/// First line of every file `wol hw boot` owns; files without it are the user's and left alone.
pub const HEADER: &str =
    "# Written by `wol hw boot` for this machine's hardware (every boot). Do not edit.\n";

/// What `wol hw boot` does: set tunables and load modules now, write tunables for the next boot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BootPlan {
    /// kenv(1) before `kldload`: kernel tunables (dotted names) of devices whose modules load now;
    /// a module reads them only at load, and loader.conf applies them from the next boot.
    pub kenv: Vec<(String, String)>,
    pub kldload: Vec<String>,
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
    let mut p = BootPlan {
        kldload: dedup(
            snap.devices
                .iter()
                .flat_map(|d| d.missing_klds.iter().cloned()),
        ),
        kenv: Vec::new(),
        writes: Vec::new(),
    };
    for d in snap.devices.iter().filter(|d| !d.missing_klds.is_empty()) {
        for (k, v) in d.loader.iter().filter_map(|l| l.split_once('=')) {
            let kv = (k.to_owned(), v.trim_matches('"').to_owned());
            if k.contains('.') && !p.kenv.contains(&kv) {
                p.kenv.push(kv);
            }
        }
    }
    let body = |lines: &[String]| {
        let mut s = HEADER.to_owned();
        for l in lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    };
    let loader = dedup(snap.devices.iter().flat_map(|d| d.loader.iter().cloned()));
    let want = body(&loader);
    match current(LOADER_CONF) {
        Some(cur) if cur == want => {}
        None if loader.is_empty() => {}
        _ => p.writes.push((LOADER_CONF.into(), want)),
    }
    let rc = dedup(snap.devices.iter().flat_map(|d| d.rc.iter().cloned()));
    let mut services: Vec<&str> = Vec::new();
    for l in &rc {
        if let Some((svc, _)) = l.split_once('=').and_then(|(var, _)| var.rsplit_once('_'))
            && !services.contains(&svc)
        {
            services.push(svc);
        }
    }
    for svc in services {
        let lines: Vec<String> = rc
            .iter()
            .filter(|l| l.starts_with(&format!("{svc}_")))
            .cloned()
            .collect();
        let path = format!("{RC_CONF_DIR}/{svc}");
        let want = body(&lines);
        match current(&path) {
            Some(cur) if cur == want || !cur.starts_with(HEADER) => {}
            _ => p.writes.push((path, want)),
        }
    }
    p
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
