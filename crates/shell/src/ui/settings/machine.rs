//! What the panes read from the machine, behind one trait: [`Live`] asks the system through
//! `wol_core` readers (always off the UI thread, see `super::later`); [`Demo`] is a fixed laptop so
//! golden renders and screenshots never depend on the build machine.
use std::sync::Arc;
use wol_core::bluetooth::{BtDevice, BtKind, BtState};
use wol_core::netinfo::{IfKind, Interface, NetInfo};
use wol_core::power::{BatteryInfo, Charge, Lid, Profile};
use wol_core::printers::Printer;
use wol_core::storage::{Category, Storage};
use wol_core::system::Host as Os;

pub use shared::hardware::{Device, HardwareSnapshot, Kind, State};

/// Sharing facts (General > Sharing, Privacy & Security). None = cannot tell here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sharing {
    pub remote_login: Option<bool>,
    pub firewall: Option<bool>,
}

/// Kernel facts for About.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kernel {
    /// `kern.ident`: "WolKernel" (kernel/conf/WolKernel; "WOLOS" before the rename).
    pub ident: String,
    /// `kern.osrelease`: "15.1-RELEASE-p1".
    pub release: String,
}

impl Kernel {
    /// "WolKernel 15.1" on the WolOS kernel, else "FreeBSD 15.1-RELEASE (GENERIC)".
    pub fn label(&self) -> String {
        let base = self.release.split('-').next().unwrap_or("");
        if self.ident == "WolKernel" || self.ident == "WOLOS" {
            format!("WolKernel {base}").trim().to_string()
        } else if self.release.is_empty() {
            self.ident.clone()
        } else {
            format!("FreeBSD {} ({})", self.release, self.ident)
        }
    }
}

pub trait Machine: Send + Sync {
    fn storage(&self) -> Option<Storage>;
    fn battery(&self) -> Option<BatteryInfo>;
    /// Energy profile (hwpstate EPP); None without one.
    fn profile(&self) -> Option<Profile>;
    fn lid(&self) -> Option<Lid>;
    fn zones(&self) -> Vec<String>;
    fn zone(&self) -> String;
    fn ntp(&self) -> Option<bool>;
    fn net(&self) -> NetInfo;
    fn known_networks(&self) -> Vec<String>;
    fn bluetooth(&self) -> BtState;
    fn printers(&self) -> Option<Vec<Printer>>;
    /// Device URIs CUPS can see (slow: probes the network).
    fn discover_printers(&self) -> Vec<String>;
    fn hardware(&self) -> HardwareSnapshot;
    fn sharing(&self) -> Sharing;
    /// Compositor consent grants: (app path, capability) (`wol_core::consent`).
    fn consent(&self) -> Vec<(String, String)>;
    fn kernel(&self) -> Kernel;
    /// Energy mode, CPU steering and what the power plan does now (`wol_core::power::policy`).
    fn power(&self) -> Option<wol_core::power::policy::Status>;
    /// Firewall, remote login and kernel protections (`wol_core::security`).
    fn security(&self) -> Option<wol_core::security::Status>;
    /// The daily update check's findings; None before the first check.
    fn updates(&self) -> Option<wol_core::update::check::Updates>;
    /// /etc/passwd text (accounts and their login shells).
    fn passwd(&self) -> String;
}

/// The running system.
pub struct Live(pub Arc<dyn Os>);

impl Live {
    pub fn new() -> Self {
        Live(Arc::new(wol_core::system::PosixHost))
    }
    fn os(&self) -> &dyn Os {
        &*self.0
    }
    fn wlan(&self) -> String {
        self.net()
            .interfaces
            .into_iter()
            .find(|i| i.kind == IfKind::WiFi)
            .map_or_else(|| "wlan0".into(), |i| i.name)
    }
}

impl Default for Live {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine for Live {
    fn storage(&self) -> Option<Storage> {
        wol_core::storage::read(self.os())
    }
    fn battery(&self) -> Option<BatteryInfo> {
        wol_core::power::battery(self.os())
    }
    fn profile(&self) -> Option<Profile> {
        wol_core::power::profile(self.os())
    }
    fn lid(&self) -> Option<Lid> {
        wol_core::power::lid(self.os())
    }
    fn zones(&self) -> Vec<String> {
        wol_core::region::zones(self.os())
    }
    fn zone(&self) -> String {
        wol_core::region::current_zone(self.os())
    }
    fn ntp(&self) -> Option<bool> {
        wol_core::region::ntp_enabled(self.os())
    }
    fn net(&self) -> NetInfo {
        wol_core::netinfo::read(self.os())
    }
    fn known_networks(&self) -> Vec<String> {
        wol_core::netinfo::known_networks(self.os(), &self.wlan())
    }
    fn bluetooth(&self) -> BtState {
        wol_core::bluetooth::read(self.os())
    }
    fn printers(&self) -> Option<Vec<Printer>> {
        wol_core::printers::read(self.os())
    }
    fn discover_printers(&self) -> Vec<String> {
        wol_core::printers::discover(self.os())
    }
    fn hardware(&self) -> HardwareSnapshot {
        // The boot step's snapshot is fresh enough and instant; detect only without one.
        self.os()
            .read_text(shared::hardware::SNAPSHOT_PATH)
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| wol_core::hardware::read(self.os()))
    }
    fn sharing(&self) -> Sharing {
        Sharing {
            remote_login: wol_core::sharing::remote_login(self.os()),
            firewall: wol_core::sharing::firewall(self.os()),
        }
    }
    fn consent(&self) -> Vec<(String, String)> {
        wol_core::consent::read(self.os())
    }
    fn kernel(&self) -> Kernel {
        Kernel {
            ident: self.os().sysctl_string("kern.ident").unwrap_or_default(),
            release: self
                .os()
                .sysctl_string("kern.osrelease")
                .unwrap_or_default(),
        }
    }
    fn power(&self) -> Option<wol_core::power::policy::Status> {
        use wol_core::power::policy;
        let os = self.os();
        let facts = policy::facts(&|argv: &[&str]| os.run(argv));
        let state = policy::State::parse(os.read_text(policy::STATE_FILE).as_deref());
        Some(policy::status(&facts, &state))
    }
    fn security(&self) -> Option<wol_core::security::Status> {
        let os = self.os();
        Some(wol_core::security::status(&|argv: &[&str]| os.run(argv)))
    }
    fn updates(&self) -> Option<wol_core::update::check::Updates> {
        use wol_core::update::check::{DIR, USERLAND_ARGV, Updates};
        let os = self.os();
        let f = |n: &str| os.read_text(&format!("{DIR}/{n}")).unwrap_or_default();
        let u = Updates::parse(&f("checked"), &f("pkg"), &f("audit"), &f("base"))?;
        let userland = u.base.as_ref().and_then(|_| os.run(&USERLAND_ARGV));
        Some(u.installed(userland.as_deref()))
    }
    fn passwd(&self) -> String {
        self.os().read_text("/etc/passwd").unwrap_or_default()
    }
}

/// A 14-inch laptop on WolOS: the same facts every render.
pub struct Demo;

const GB: u64 = 1_000_000_000;

impl Machine for Demo {
    fn storage(&self) -> Option<Storage> {
        let c = |n: &str, gb: u64| Category {
            name: n.into(),
            bytes: gb * GB,
        };
        Some(Storage {
            name: "zroot".into(),
            total: 512 * GB,
            used: 143 * GB,
            categories: vec![
                c("Documents", 61),
                c("Apps", 38),
                c("System", 24),
                c("Restore points", 12),
                c("Other", 8),
            ],
            encrypted: Some(true),
            snapshots: 12 * GB,
            boot_environments: 3,
            zfs: true,
        })
    }
    fn battery(&self) -> Option<BatteryInfo> {
        Some(BatteryInfo {
            percent: 82,
            charge: Charge::Discharging,
            health: Some(94),
            cycles: Some(212),
            minutes_left: Some(205),
            design_mwh: Some(57_000),
            full_mwh: Some(53_580),
        })
    }
    fn profile(&self) -> Option<Profile> {
        Some(Profile::Automatic)
    }
    fn lid(&self) -> Option<Lid> {
        Some(Lid::Sleep)
    }
    fn zones(&self) -> Vec<String> {
        [
            "America/New_York",
            "America/Sao_Paulo",
            "Europe/Lisbon",
            "Europe/London",
            "Asia/Tokyo",
        ]
        .map(String::from)
        .to_vec()
    }
    fn zone(&self) -> String {
        "Europe/Lisbon".into()
    }
    fn ntp(&self) -> Option<bool> {
        Some(true)
    }
    fn net(&self) -> NetInfo {
        NetInfo {
            interfaces: vec![
                Interface {
                    name: "wlan0".into(),
                    kind: IfKind::WiFi,
                    up: true,
                    mac: "a4:c3:f0:12:7b:9e".into(),
                    ipv4: vec!["192.168.1.24/24".into()],
                    ipv6: vec!["2001:db8::24/64".into()],
                },
                Interface {
                    name: "em0".into(),
                    kind: IfKind::Ethernet,
                    up: false,
                    mac: "8c:16:45:aa:01:3c".into(),
                    ..Default::default()
                },
            ],
            dns: vec!["192.168.1.1".into(), "9.9.9.9".into()],
            search: vec!["home.arpa".into()],
            gateway: Some("192.168.1.1".into()),
        }
    }
    fn known_networks(&self) -> Vec<String> {
        vec!["Wonderland".into(), "Mad Hatter Café".into()]
    }
    fn bluetooth(&self) -> BtState {
        let d = |a: &str, n: &str, kind, connected| BtDevice {
            addr: a.into(),
            name: n.into(),
            kind,
            paired: true,
            connected,
        };
        BtState {
            present: true,
            powered: true,
            devices: vec![
                // Same devices as the Fake system's paired list (Control Center goldens).
                d("aa:bb:cc:dd:ee:00", "AirPods Pro", BtKind::Audio, true),
                d("aa:bb:cc:dd:ee:01", "MX Keys", BtKind::Keyboard, true),
                d("aa:bb:cc:dd:ee:02", "Magic Trackpad", BtKind::Mouse, false),
            ],
        }
    }
    fn printers(&self) -> Option<Vec<Printer>> {
        Some(vec![
            Printer {
                name: "Brother_HL_L2350DW".into(),
                uri: "ipp://192.168.1.40/ipp/print".into(),
                default: true,
                state: "Idle".into(),
            },
            Printer {
                name: "Office_Laser".into(),
                uri: "ipp://192.168.1.41/ipp/print".into(),
                default: false,
                state: "Stopped".into(),
            },
        ])
    }
    fn discover_printers(&self) -> Vec<String> {
        vec![
            "dnssd://HP%20Color%20LaserJet%20MFP._ipp._tcp.local/".into(),
            "ipp://192.168.1.40/ipp/print".into(),
        ]
    }
    fn hardware(&self) -> HardwareSnapshot {
        let dev = |kind, name: &str, driver: Option<&str>, state, hint: &str| Device {
            kind,
            name: name.into(),
            driver: driver.map(String::from),
            state,
            hint: hint.into(),
            ..Default::default()
        };
        HardwareSnapshot {
            devices: vec![
                dev(
                    Kind::Cpu,
                    "Intel Core i5-1240P",
                    Some("hwpstate_intel0"),
                    State::Ready,
                    "",
                ),
                dev(
                    Kind::Gpu,
                    "Intel Iris Xe Graphics",
                    Some("drmn0"),
                    State::Ready,
                    "",
                ),
                dev(
                    Kind::Wifi,
                    "Intel Wi-Fi 6E AX211",
                    Some("iwlwifi0"),
                    State::Ready,
                    "",
                ),
                dev(
                    Kind::Bluetooth,
                    "Intel Bluetooth",
                    Some("ubt0"),
                    State::Ready,
                    "",
                ),
                dev(
                    Kind::Audio,
                    "Realtek ALC287 audio",
                    Some("hdac0"),
                    State::Ready,
                    "",
                ),
                Device {
                    missing_packages: vec!["webcamd".into()],
                    ..dev(
                        Kind::Webcam,
                        "Integrated Camera",
                        None,
                        State::NeedsDriver,
                        "Install webcamd to use the camera.",
                    )
                },
                Device {
                    missing_packages: vec!["rtsx-firmware".into()],
                    ..dev(
                        Kind::CardReader,
                        "Realtek card reader",
                        Some("rtsx0"),
                        State::NeedsFirmware,
                        "Install its firmware to read SD cards.",
                    )
                },
                dev(
                    Kind::Input,
                    "Synaptics touchpad",
                    Some("iichid0"),
                    State::Ready,
                    "",
                ),
            ],
        }
    }
    fn sharing(&self) -> Sharing {
        Sharing {
            remote_login: Some(false),
            firewall: Some(true),
        }
    }
    fn consent(&self) -> Vec<(String, String)> {
        vec![
            ("/usr/local/bin/obs".into(), "screen-capture".into()),
            ("/usr/local/bin/ydotoold".into(), "input-injection".into()),
        ]
    }
    fn kernel(&self) -> Kernel {
        Kernel {
            ident: "WolKernel".into(),
            release: "15.1-RELEASE".into(),
        }
    }
    fn power(&self) -> Option<wol_core::power::policy::Status> {
        use wol_core::power::policy::{Cpu, Mode, Source, Status};
        Some(Status {
            source: Source::Battery,
            mode: Mode::Auto,
            lid: Some(Lid::Sleep),
            battery: Some(82),
            cpu: Cpu::Epp,
            epp: Some(128),
            cx_lowest: Some("Cmax".into()),
            sleep: true,
            wifi_power_save: vec!["wlan0".into()],
            charge_limit: None,
        })
    }
    fn security(&self) -> Option<wol_core::security::Status> {
        Some(wol_core::security::Status {
            firewall: Some(true),
            firewall_running: Some(true),
            remote_login: Some(false),
            system_log_private: true,
            hardening: wol_core::security::HARDENING
                .iter()
                .map(|(k, v, _)| wol_core::security::Check {
                    key: (*k).into(),
                    want: (*v).into(),
                    now: Some((*v).into()),
                })
                .collect(),
        })
    }
    fn updates(&self) -> Option<wol_core::update::check::Updates> {
        Some(wol_core::update::check::Updates {
            checked: wol_core::journal::now().saturating_sub(3 * 3600),
            packages: vec!["firefox".into(), "curl".into(), "mesa-dri".into()],
            security: vec!["curl".into()],
            base: Some("15.1-RELEASE-p3".into()),
        })
    }
    fn passwd(&self) -> String {
        "root:*:0:0:Charlie &:/root:/bin/sh\n\
         alice:*:1001:1001:Alice Liddell:/home/alice:/usr/local/bin/zsh\n\
         dinah:*:1002:1002:Dinah:/home/dinah:/usr/local/bin/zsh\n"
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_label() {
        let k = |i: &str, r: &str| {
            Kernel {
                ident: i.into(),
                release: r.into(),
            }
            .label()
        };
        assert_eq!(k("WolKernel", "15.1-RELEASE-p6"), "WolKernel 15.1");
        assert_eq!(k("WOLOS", "15.1-RELEASE-p2"), "WolKernel 15.1");
        assert_eq!(
            k("GENERIC", "15.1-RELEASE"),
            "FreeBSD 15.1-RELEASE (GENERIC)"
        );
        assert_eq!(k("GENERIC", ""), "GENERIC");
    }

    #[test]
    fn sharing_reads_rc_conf() {
        let h = wol_core::system::fake::FakeHost::new(true);
        h.command("sysrc -n sshd_enable", "YES\n")
            .command("sysrc -n pf_enable", "NO\n");
        let live = Live(Arc::new(h));
        assert_eq!(
            live.sharing(),
            Sharing {
                remote_login: Some(true),
                firewall: Some(false)
            }
        );
        let linux = Live(Arc::new(wol_core::system::fake::FakeHost::new(false)));
        assert_eq!(linux.sharing(), Sharing::default());
    }
}
