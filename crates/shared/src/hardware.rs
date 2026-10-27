//! Hardware status contract (`wol hw list --json`, [`SNAPSHOT_PATH`], Settings, scripts, AI helpers).
//! Built by `wol_core::hardware` from the catalogue; one [`Device`] per detected piece of hardware.
use serde::{Deserialize, Serialize};

/// Written by `wol hw boot` (rc.d `wolos_hw`) on every boot; readable by everyone.
pub const SNAPSHOT_PATH: &str = "/var/run/wolos-hw.json";

/// Written by `wol hw online` (network up, root) after it installed or tried to install the
/// packages the catalogue was missing; readable by everyone, gone at the next boot.
pub const ONLINE_PATH: &str = "/var/run/wolos-hw-online.json";

/// One `wol hw online` attempt (`wol_core::hardware::online`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OnlineInstall {
    /// Unix time of the attempt: the session shows each attempt once.
    pub at: u64,
    pub packages: Vec<String>,
    /// `pkg install` succeeded: the driver loads at the next boot.
    pub ok: bool,
    /// A GPU needed the packages (the graphics driver).
    pub graphics: bool,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Cpu,
    Gpu,
    Wifi,
    Ethernet,
    Bluetooth,
    Audio,
    Webcam,
    Input,
    CardReader,
    Backlight,
    #[default]
    Other,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Cpu => "cpu",
            Kind::Gpu => "gpu",
            Kind::Wifi => "wifi",
            Kind::Ethernet => "ethernet",
            Kind::Bluetooth => "bluetooth",
            Kind::Audio => "audio",
            Kind::Webcam => "webcam",
            Kind::Input => "input",
            Kind::CardReader => "card-reader",
            Kind::Backlight => "backlight",
            Kind::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    #[default]
    Ready,
    /// A module to load (`wol hw fix`) or a driver package to install.
    NeedsDriver,
    /// Driver present, firmware package missing.
    NeedsFirmware,
    /// No driver for this device in this release.
    Unsupported,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Ready => "ready",
            State::NeedsDriver => "needs-driver",
            State::NeedsFirmware => "needs-firmware",
            State::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Device {
    pub kind: Kind,
    /// Human name (catalogue name, else the bus's description).
    pub name: String,
    /// `pci` | `usb` | `acpi` | `cpu` | `backlight`.
    pub bus: String,
    /// `vvvv:dddd` for PCI/USB, ACPI id, CPU vendor.
    pub id: String,
    /// `pci0:0:20:3`, `ugen0.2`, `cpu0`, `/dev/backlight/backlight0`.
    pub location: String,
    /// Attached driver unit (`iwlwifi0`, `drmn0`, `ubt0`).
    pub driver: Option<String>,
    /// Kernel modules the catalogue loads for it.
    pub klds: Vec<String>,
    /// Firmware packages.
    pub firmware: Vec<String>,
    /// Driver / userland packages (`nvidia-drm-kmod`, `webcamd`).
    pub packages: Vec<String>,
    /// Modules not loaded yet (on disk; `wol hw fix` loads them).
    pub missing_klds: Vec<String>,
    /// Firmware or driver packages not installed.
    pub missing_packages: Vec<String>,
    /// loader.conf lines for the next boot.
    pub loader: Vec<String>,
    /// rc.conf lines (`powerd_enable="NO"`).
    pub rc: Vec<String>,
    pub notes: Vec<String>,
    pub state: State,
    /// One line: what to do, empty when ready.
    pub hint: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HardwareSnapshot {
    pub devices: Vec<Device>,
}

impl HardwareSnapshot {
    /// Devices that are not ready.
    pub fn problems(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter().filter(|d| d.state != State::Ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_names_are_stable_and_partial_input_parses() {
        let d = Device {
            kind: Kind::CardReader,
            state: State::NeedsFirmware,
            ..Default::default()
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["kind"], "card-reader");
        assert_eq!(v["state"], "needs-firmware");
        assert_eq!(Kind::CardReader.name(), "card-reader");
        assert_eq!(State::NeedsFirmware.name(), "needs-firmware");
        let s: HardwareSnapshot =
            serde_json::from_str(r#"{"devices":[{"kind":"wifi","name":"AX211"}]}"#).unwrap();
        assert_eq!(s.devices[0].kind, Kind::Wifi);
        assert_eq!(s.devices[0].state, State::Ready);
        assert_eq!(s.problems().count(), 0);
    }
}
