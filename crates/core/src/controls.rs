//! Desktop controls for Control Center and Settings: Wi-Fi, audio, brightness, Bluetooth, displays, Night Shift.
//! Per topic: pure parser (fixture-tested) + fixed argv per platform. Reads go through [`Host`] (2 s timeout);
//! callers cache and call off the UI thread. Root changes go through wol-admind ([`shared::admin`]).
//! Secrets reach tools on stdin ([`Controls::stdin`]), never argv.
use crate::system::Host;
use shared::admin::{AdminAction, AdminReply, AdminRequest};
use std::sync::Arc;

mod center;
pub use center::*;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WifiNet {
    pub ssid: String,
    /// 0..=100.
    pub signal: u8,
    pub secure: bool,
    pub connected: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Volume {
    pub percent: u8,
    pub muted: bool,
}

/// Audio output device: `id` is what the setter takes (sink name / pcmN).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Output {
    pub id: String,
    pub name: String,
    pub default: bool,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Monitor {
    pub name: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub refresh: f64,
    pub scale: f64,
    /// "WxH@R" strings as Hyprland lists them.
    pub modes: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlError {
    Unavailable,
    NeedsAuth,
    Denied,
    NotAdmin,
    Invalid,
    Failed,
    /// Wi-Fi join: the network refused the password.
    WrongPassword,
}

/// wol-admind reply -> control result.
pub fn from_reply(r: AdminReply) -> Result<(), ControlError> {
    {
        match r {
            AdminReply::Ok => Ok(()),
            AdminReply::NeedsAuth => Err(ControlError::NeedsAuth),
            AdminReply::Denied => Err(ControlError::Denied),
            AdminReply::NotAdmin => Err(ControlError::NotAdmin),
            AdminReply::Invalid => Err(ControlError::Invalid),
            AdminReply::Failed => Err(ControlError::Failed),
        }
    }
}

// ---- Linux parsers ----

/// `nmcli -t -f IN-USE,SIGNAL,SECURITY,SSID dev wifi list`: SSID last (may contain `\:`), hidden networks dropped,
/// one row per SSID (connected or strongest wins), sorted connected first then by signal.
pub fn parse_nmcli_wifi(text: &str) -> Vec<WifiNet> {
    let mut v: Vec<WifiNet> = Vec::new();
    for line in text.lines() {
        let mut f = line.splitn(4, ':');
        let (Some(used), Some(sig), Some(sec), Some(ssid)) =
            (f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let ssid = ssid.replace("\\:", ":");
        if ssid.is_empty() {
            continue;
        }
        let n = WifiNet {
            ssid,
            signal: sig.trim().parse::<u8>().unwrap_or(0).min(100),
            secure: !sec.trim().is_empty() && sec.trim() != "--",
            connected: used.trim() == "*",
        };
        match v.iter_mut().find(|x| x.ssid == n.ssid) {
            Some(x) => {
                if n.connected || (!x.connected && n.signal > x.signal) {
                    *x = n;
                }
            }
            None => v.push(n),
        }
    }
    sort_nets(&mut v);
    v
}

fn sort_nets(v: &mut [WifiNet]) {
    v.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.signal.cmp(&a.signal))
            .then(a.ssid.cmp(&b.ssid))
    });
}

/// `wpctl get-volume @DEFAULT_AUDIO_SINK@`: "Volume: 0.40 [MUTED]".
pub fn parse_wpctl_volume(text: &str) -> Option<Volume> {
    let rest = text.trim().strip_prefix("Volume:")?.trim();
    let num = rest.split_whitespace().next()?.parse::<f64>().ok()?;
    Some(Volume {
        percent: (num * 100.0).round().clamp(0.0, 100.0) as u8,
        muted: rest.contains("[MUTED]"),
    })
}

/// `pactl list sinks` (Name:/Description: lines) + `pactl get-default-sink`.
pub fn parse_pactl_sinks(text: &str, default: &str) -> Vec<Output> {
    let mut v: Vec<Output> = Vec::new();
    for l in text.lines().map(str::trim) {
        if let Some(n) = l.strip_prefix("Name:") {
            v.push(Output {
                id: n.trim().into(),
                name: n.trim().into(),
                default: n.trim() == default.trim(),
            });
        } else if let (Some(d), Some(last)) = (l.strip_prefix("Description:"), v.last_mut()) {
            last.name = d.trim().into();
        }
    }
    v
}

/// `bluetoothctl show`: "Powered: yes". None without a controller.
pub fn parse_bluetoothctl(text: &str) -> Option<bool> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("Powered:"))
        .map(|v| v.trim() == "yes")
}

/// sysfs backlight: brightness / max_brightness -> percent.
pub fn backlight_percent(cur: &str, max: &str) -> Option<u8> {
    let (c, m) = (
        cur.trim().parse::<u64>().ok()?,
        max.trim().parse::<u64>().ok()?,
    );
    (m > 0).then(|| (c.min(m) * 100 / m) as u8)
}

// ---- FreeBSD parsers ----

/// `wpa_cli -i wlan0 scan_results`: header line, then `bssid freq signal flags ssid` (tab-separated, dBm).
pub fn parse_scan_results(text: &str, connected: Option<&str>) -> Vec<WifiNet> {
    let mut v: Vec<WifiNet> = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 5 || f[4].is_empty() {
            continue;
        }
        let Ok(dbm) = f[2].trim().parse::<i32>() else {
            continue;
        };
        let n = WifiNet {
            ssid: f[4].into(),
            signal: crate::wifi::signal_percent(dbm) as u8,
            secure: ["WPA", "WEP", "RSN", "SAE"]
                .iter()
                .any(|k| f[3].contains(k)),
            connected: connected == Some(f[4]),
        };
        match v.iter_mut().find(|x| x.ssid == n.ssid) {
            Some(x) if n.signal > x.signal => *x = n,
            Some(_) => {}
            None => v.push(n),
        }
    }
    sort_nets(&mut v);
    v
}

/// `wpa_cli -i wlan0 status`: ssid when associated.
pub fn parse_wpa_status(text: &str) -> Option<String> {
    let p = crate::system::pairs(text, '=');
    (p.get("wpa_state") == Some(&"COMPLETED"))
        .then(|| p.get("ssid").map(|s| s.to_string()))
        .flatten()
}

/// `mixer -o vol.volume vol.mute` (FreeBSD 14+): "vol.volume=0.75:0.75" "vol.mute=0".
pub fn parse_mixer(text: &str) -> Option<Volume> {
    let p = crate::system::pairs(text, '=');
    let left = p
        .get("vol.volume")?
        .split(':')
        .next()?
        .parse::<f64>()
        .ok()?;
    Some(Volume {
        percent: (left * 100.0).round().clamp(0.0, 100.0) as u8,
        muted: p.get("vol.mute").is_some_and(|m| *m == "1"),
    })
}

/// `/dev/sndstat`: "pcm0: <Realtek ALC269 (Analog)> (play/rec) default".
pub fn parse_sndstat(text: &str) -> Vec<Output> {
    text.lines()
        .filter(|l| l.starts_with("pcm") && l.contains("play"))
        .filter_map(|l| {
            let (id, rest) = l.split_once(':')?;
            let name = rest.split_once('<')?.1.split_once('>')?.0;
            Some(Output {
                id: id.trim().into(),
                name: name.trim().into(),
                default: rest.trim_end().ends_with("default"),
            })
        })
        .collect()
}

/// `backlight -q` (FreeBSD 13+): "75".
pub fn parse_backlight(text: &str) -> Option<u8> {
    let t = text.trim();
    let n = t.rsplit([' ', ':']).next()?.trim();
    n.parse::<u8>().ok().map(|p| p.min(100))
}

// ---- Hyprland ----

/// `hyprctl -j monitors all`.
pub fn parse_hypr_monitors(json: &str) -> Vec<Monitor> {
    let Ok(serde_json::Value::Array(a)) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    a.iter()
        .filter_map(|m| {
            let s = |k: &str| m.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let n = |k: &str| m.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
            let name = s("name");
            (!name.is_empty()).then(|| Monitor {
                name,
                description: s("description"),
                width: n("width") as u32,
                height: n("height") as u32,
                refresh: n("refreshRate"),
                scale: n("scale"),
                modes: m
                    .get("availableModes")
                    .and_then(|x| x.as_array())
                    .map(|v| {
                        v.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// "1920x1080@60.00Hz" -> (1920, 1080, 60.0).
pub fn parse_mode(mode: &str) -> Option<(u32, u32, f64)> {
    let (wh, r) = mode.trim_end_matches("Hz").split_once('@')?;
    let (w, h) = wh.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?, r.parse().ok()?))
}

/// Hyprland `monitor=` value: "eDP-1,1920x1080@60.00,auto,1.25".
pub fn monitor_rule(name: &str, w: u32, h: u32, refresh: f64, scale: f64) -> Option<String> {
    let ok = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && (0.5..=3.0).contains(&scale)
        && w > 0
        && h > 0;
    ok.then(|| format!("{name},{w}x{h}@{refresh:.2},auto,{scale}"))
}

pub const NIGHT_SHIFT_KELVIN: u32 = 4500;

/// How long a FreeBSD Wi-Fi join waits for the network, and how often it looks.
pub const JOIN_WAIT: std::time::Duration = std::time::Duration::from_secs(20);
const JOIN_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// Platform reads and writes. `stdin` runs argv with input on stdin (secrets); `admin` sends one wol-admind request.
pub type StdinRunner = dyn Fn(&[&str], &str) -> Option<String> + Send + Sync;
pub type AdminSender = dyn Fn(&AdminRequest, &str) -> Option<AdminReply> + Send + Sync;

pub struct Controls {
    pub host: Arc<dyn Host>,
    pub stdin: Arc<StdinRunner>,
    pub admin: Arc<AdminSender>,
    /// FreeBSD wireless interface.
    pub wlan: String,
}

fn sv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

impl Controls {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self {
            host,
            stdin: Arc::new(|argv, input| {
                run_with_input(argv, input, std::time::Duration::from_secs(60))
            }),
            admin: Arc::new(|req, pw| {
                admin_request(std::path::Path::new(shared::admin::SOCKET), req, pw)
            }),
            wlan: "wlan0".into(),
        }
    }

    fn bsd(&self) -> bool {
        self.host.is_freebsd()
    }

    fn run(&self, argv: &[&str]) -> Option<String> {
        self.host.run(argv)
    }

    fn admin(&self, action: AdminAction, args: &[&str], pw: &str) -> Result<(), ControlError> {
        self.admin_request(
            &AdminRequest {
                action,
                args: sv(args),
            },
            pw,
        )
    }

    /// Any allow-listed wol-admind request (Settings: one action for every root change). Refused
    /// here when [`crate::admin::validate`] refuses it; Unavailable when wol-admind is not running.
    pub fn admin_request(&self, req: &AdminRequest, pw: &str) -> Result<(), ControlError> {
        if crate::admin::validate(req).is_some() {
            return Err(ControlError::Invalid);
        }
        match (self.admin)(req, pw) {
            Some(r) => from_reply(r),
            None => Err(ControlError::Unavailable),
        }
    }

    fn ok(&self, out: Option<String>) -> Result<(), ControlError> {
        out.map(|_| ()).ok_or(ControlError::Failed)
    }

    // ---- Wi-Fi ----

    pub fn wifi_enabled(&self) -> Option<bool> {
        if self.bsd() {
            let o = self.run(&["ifconfig", &self.wlan])?;
            Some(o.lines().next().is_some_and(|l| l.contains("<UP")))
        } else {
            self.run(&["nmcli", "radio", "wifi"])
                .map(|o| o.trim() == "enabled")
        }
    }

    pub fn wifi_networks(&self) -> Vec<WifiNet> {
        if self.bsd() {
            let cur = self
                .run(&["wpa_cli", "-i", &self.wlan, "status"])
                .and_then(|s| parse_wpa_status(&s));
            self.run(&["wpa_cli", "-i", &self.wlan, "scan_results"])
                .map(|s| parse_scan_results(&s, cur.as_deref()))
                .unwrap_or_default()
        } else {
            self.run(&[
                "nmcli",
                "-t",
                "-f",
                "IN-USE,SIGNAL,SECURITY,SSID",
                "dev",
                "wifi",
                "list",
                "--rescan",
                "no",
            ])
            .map(|s| parse_nmcli_wifi(&s))
            .unwrap_or_default()
        }
    }

    pub fn set_wifi(&self, on: bool, pw: &str) -> Result<(), ControlError> {
        let v = if on { "on" } else { "off" };
        if self.bsd() {
            self.admin(AdminAction::WifiPower, &[&self.wlan, v], pw)
        } else {
            self.ok(self.run(&["nmcli", "radio", "wifi", v]))
        }
    }

    /// Joins (slow: call off the UI thread). Password on stdin (Linux) or inside the admin request (FreeBSD).
    pub fn wifi_join(
        &self,
        ssid: &str,
        password: Option<&str>,
        admin_pw: &str,
    ) -> Result<(), ControlError> {
        if crate::wifi::validate(ssid, password).is_some() {
            return Err(ControlError::Invalid);
        }
        if self.bsd() {
            let mut a = vec![self.wlan.as_str(), ssid];
            if let Some(p) = password {
                a.push(p);
            }
            self.admin(AdminAction::WifiJoin, &a, admin_pw)?;
            return self.wait_joined(ssid, password.is_some());
        }
        let out = match password {
            Some(p) => (self.stdin)(
                &[
                    "nmcli",
                    "dev",
                    "wifi",
                    "connect",
                    ssid,
                    "passwd-file",
                    "/dev/stdin",
                ],
                &format!("802-11-wireless-security.psk:{p}\n"),
            ),
            None => self
                .host
                .run_slow(&["nmcli", "dev", "wifi", "connect", ssid]),
        };
        out.map(|_| ()).ok_or(if password.is_some() {
            ControlError::WrongPassword
        } else {
            ControlError::Failed
        })
    }

    /// FreeBSD, after wol-admind reloaded wpa_supplicant: Ok once `ssid` is joined, WrongPassword
    /// when a secured network refused us, Failed after [`JOIN_WAIT`]. Unreadable wpa_supplicant
    /// (not in its group): Ok, as before the wait existed.
    fn wait_joined(&self, ssid: &str, secured: bool) -> Result<(), ControlError> {
        let start = std::time::Instant::now();
        loop {
            let Some(status) = self.run(&["wpa_cli", "-i", &self.wlan, "status"]) else {
                return Ok(());
            };
            let nets = self
                .run(&["wpa_cli", "-i", &self.wlan, "list_networks"])
                .unwrap_or_default();
            match crate::wifi::join_state(&status, &nets, ssid) {
                crate::wifi::JoinState::Joined => return Ok(()),
                crate::wifi::JoinState::Refused if secured => {
                    return Err(ControlError::WrongPassword);
                }
                crate::wifi::JoinState::Refused => return Err(ControlError::Failed),
                crate::wifi::JoinState::Trying if start.elapsed() >= JOIN_WAIT => {
                    return Err(ControlError::Failed);
                }
                crate::wifi::JoinState::Trying => std::thread::sleep(JOIN_POLL),
            }
        }
    }

    // ---- Audio ----

    pub fn volume(&self) -> Option<Volume> {
        if self.bsd() {
            self.run(&["mixer", "-o", "vol.volume", "vol.mute"])
                .and_then(|s| parse_mixer(&s))
        } else {
            self.run(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"])
                .and_then(|s| parse_wpctl_volume(&s))
        }
    }

    pub fn set_volume(&self, percent: u8) -> Result<(), ControlError> {
        let p = percent.min(100);
        if self.bsd() {
            self.ok(self.run(&["mixer", &format!("vol.volume={:.2}", p as f64 / 100.0)]))
        } else {
            self.ok(self.run(&[
                "wpctl",
                "set-volume",
                "@DEFAULT_AUDIO_SINK@",
                &format!("{p}%"),
            ]))
        }
    }

    pub fn set_muted(&self, muted: bool) -> Result<(), ControlError> {
        let m = if muted { "1" } else { "0" };
        if self.bsd() {
            self.ok(self.run(&["mixer", &format!("vol.mute={m}")]))
        } else {
            self.ok(self.run(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", m]))
        }
    }

    pub fn outputs(&self) -> Vec<Output> {
        if self.bsd() {
            self.host
                .read_text("/dev/sndstat")
                .map(|s| parse_sndstat(&s))
                .unwrap_or_default()
        } else {
            let def = self.run(&["pactl", "get-default-sink"]).unwrap_or_default();
            self.run(&["pactl", "list", "sinks"])
                .map(|s| parse_pactl_sinks(&s, &def))
                .unwrap_or_default()
        }
    }

    pub fn set_output(&self, id: &str, pw: &str) -> Result<(), ControlError> {
        if self.bsd() {
            self.admin(AdminAction::AudioDefault, &[id], pw)
        } else if id.is_empty() || id.starts_with('-') {
            Err(ControlError::Invalid)
        } else {
            self.ok(self.run(&["pactl", "set-default-sink", id]))
        }
    }

    // ---- Brightness ----

    fn linux_backlight(&self) -> Option<(String, u64)> {
        let dir = self
            .host
            .list_dir("/sys/class/backlight")
            .into_iter()
            .next()?;
        let max = self
            .host
            .read_text(&format!("{dir}/max_brightness"))?
            .trim()
            .parse()
            .ok()?;
        Some((dir, max))
    }

    /// No fork on Linux (sysfs).
    pub fn brightness(&self) -> Option<u8> {
        if self.bsd() {
            return self
                .run(&["backlight", "-q"])
                .and_then(|s| parse_backlight(&s));
        }
        let (dir, max) = self.linux_backlight()?;
        backlight_percent(
            &self.host.read_text(&format!("{dir}/brightness"))?,
            &max.to_string(),
        )
    }

    /// Linux: logind `SetBrightness` (no root). FreeBSD: backlight(8).
    pub fn set_brightness(&self, percent: u8) -> Result<(), ControlError> {
        let p = percent.clamp(1, 100);
        if self.bsd() {
            return self.ok(self.run(&["backlight", &p.to_string()]));
        }
        let (dir, max) = self.linux_backlight().ok_or(ControlError::Unavailable)?;
        let dev = dir.rsplit('/').next().unwrap_or("");
        let raw = (max * p as u64 / 100).max(1).to_string();
        self.ok(self.run(&[
            "busctl",
            "call",
            "org.freedesktop.login1",
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.login1.Session",
            "SetBrightness",
            "ssu",
            "backlight",
            dev,
            &raw,
        ]))
    }

    // ---- Bluetooth ----

    pub fn bluetooth(&self) -> Option<bool> {
        if self.bsd() {
            let o = self.run(&["hccontrol", "-n", "ubt0hci", "read_scan_enable"])?;
            Some(!o.contains("disabled") && !o.trim().ends_with("0"))
        } else {
            self.run(&["bluetoothctl", "show"])
                .and_then(|s| parse_bluetoothctl(&s))
        }
    }

    pub fn set_bluetooth(&self, on: bool, pw: &str) -> Result<(), ControlError> {
        let v = if on { "on" } else { "off" };
        if self.bsd() {
            self.admin(AdminAction::Bluetooth, &[v], pw)
        } else {
            self.ok(self.run(&["bluetoothctl", "power", v]))
        }
    }

    // ---- Displays (Hyprland, both platforms) ----

    pub fn monitors(&self) -> Vec<Monitor> {
        self.run(&["hyprctl", "-j", "monitors", "all"])
            .map(|s| parse_hypr_monitors(&s))
            .unwrap_or_default()
    }

    pub fn set_monitor(&self, rule: &str) -> Result<(), ControlError> {
        self.ok(self.run(&["hyprctl", "keyword", "monitor", rule]))
    }

    /// hyprsunset IPC (`exec-once = hyprsunset`).
    pub fn set_night_shift(&self, on: bool) -> Result<(), ControlError> {
        let k = NIGHT_SHIFT_KELVIN.to_string();
        let argv: &[&str] = if on {
            &["hyprctl", "hyprsunset", "temperature", &k]
        } else {
            &["hyprctl", "hyprsunset", "identity"]
        };
        match self.run(argv) {
            Some(o) if !o.contains("error") && !o.contains("unknown") => Ok(()),
            Some(_) => Err(ControlError::Unavailable),
            None => Err(ControlError::Failed),
        }
    }

    // ---- Accounts / updates (admin) ----

    pub fn add_user(
        &self,
        name: &str,
        full_name: &str,
        admin: bool,
        password: &str,
        pw: &str,
    ) -> Result<(), ControlError> {
        let a = if admin { "yes" } else { "no" };
        self.admin(AdminAction::UserAdd, &[name, full_name, a, password], pw)
    }

    /// App Store: catalog id or package name (slow; worker thread).
    pub fn pkg_install(&self, target: &str, pw: &str) -> Result<(), ControlError> {
        self.admin(AdminAction::PkgInstall, &[target], pw)
    }

    pub fn pkg_upgrade(&self, pw: &str) -> Result<(), ControlError> {
        self.admin(AdminAction::PkgUpgrade, &[], pw)
    }

    /// Restore: start from boot environment `name` next time (`bectl activate`).
    pub fn boot_environment(&self, name: &str, pw: &str) -> Result<(), ControlError> {
        self.admin(AdminAction::BootEnvironment, &[name], pw)
    }

    /// Forgets this session's administrator authorization: the next admin action asks again.
    pub fn revoke(&self) {
        let _ = self.admin(AdminAction::Revoke, &[], "");
    }

    // ---- Settings reads (changes: `admin_request`) ----

    /// Network time on (`ntp`).
    pub fn network_time(&self) -> Option<bool> {
        crate::region::ntp_enabled(&*self.host)
    }

    /// Remote Login (sshd) on at boot (`remote-login`).
    pub fn remote_login(&self) -> Option<bool> {
        crate::sharing::remote_login(&*self.host)
    }

    /// Firewall (pf) on at boot (`firewall`).
    pub fn firewall(&self) -> Option<bool> {
        crate::sharing::firewall(&*self.host)
    }

    /// Computer name (`hostname`).
    pub fn hostname(&self) -> Option<String> {
        crate::sharing::hostname(&*self.host)
    }

    /// Lid action; None without a lid (`lid`).
    pub fn lid(&self) -> Option<crate::power::Lid> {
        crate::power::lid(&*self.host)
    }
}

/// Run argv with `input` on stdin; stdout when it exits 0 within `timeout`.
pub fn run_with_input(argv: &[&str], input: &str, timeout: std::time::Duration) -> Option<String> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let (file, args) = argv.split_first()?;
    let mut child = Command::new(file)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut i) = child.stdin.take() {
        let _ = i.write_all(input.as_bytes());
    }
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out.read_to_end(&mut s);
        s
    });
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    }?;
    let bytes = reader.join().ok()?;
    status
        .success()
        .then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// One wol-admind round trip. None when the daemon is not running. Long actions (pkg) may take minutes.
pub fn admin_request(
    sock: &std::path::Path,
    req: &AdminRequest,
    password: &str,
) -> Option<AdminReply> {
    use std::io::{Read, Write};
    let mut s = std::os::unix::net::UnixStream::connect(sock).ok()?;
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(1800)));
    s.write_all(&shared::admin::encode(req, password.as_bytes()))
        .ok()?;
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut line = String::new();
    s.take(256).read_to_string(&mut line).ok()?;
    Some(AdminReply::parse(&line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;
    use std::sync::Mutex;

    #[test]
    fn nmcli_wifi_table() {
        let t = "*:50:WPA2:Home\n :34:WPA1 WPA2:Cafe\\:Bar\n :32:WPA2:\n :70:WPA2:Cafe\\:Bar\n :20::Open\n :90:WPA2:Home\n";
        let v = parse_nmcli_wifi(t);
        assert_eq!(
            v.iter()
                .map(|n| (n.ssid.as_str(), n.signal, n.secure, n.connected))
                .collect::<Vec<_>>(),
            [
                ("Home", 50, true, true),
                ("Cafe:Bar", 70, true, false),
                ("Open", 20, false, false)
            ]
        );
    }

    #[test]
    fn audio_parsers() {
        for (t, want) in [
            ("Volume: 0.40\n", Some((40, false))),
            ("Volume: 1.00 [MUTED]\n", Some((100, true))),
            ("junk", None),
        ] {
            assert_eq!(
                parse_wpctl_volume(t).map(|v| (v.percent, v.muted)),
                want,
                "{t}"
            );
        }
        assert_eq!(
            parse_mixer("vol.volume=0.75:0.70\nvol.mute=1\n"),
            Some(Volume {
                percent: 75,
                muted: true
            })
        );
        let sinks = "Sink #43\n\tState: RUNNING\n\tName: mic_sink\n\tDescription: Mic\nSink #70\n\tName: alsa.out\n\tDescription: Built-in Audio\n";
        let o = parse_pactl_sinks(sinks, "alsa.out\n");
        assert_eq!(o.len(), 2);
        assert_eq!(
            (o[1].id.as_str(), o[1].name.as_str(), o[1].default),
            ("alsa.out", "Built-in Audio", true)
        );
        let st = "Installed devices:\npcm0: <Realtek ALC269 (Analog)> (play/rec) default\npcm1: <HDMI> (play)\npcm2: <Mic> (rec)\n";
        let o = parse_sndstat(st);
        assert_eq!(
            o.iter()
                .map(|x| (x.id.as_str(), x.name.as_str(), x.default))
                .collect::<Vec<_>>(),
            [
                ("pcm0", "Realtek ALC269 (Analog)", true),
                ("pcm1", "HDMI", false)
            ]
        );
    }

    #[test]
    fn freebsd_wifi_and_misc_parsers() {
        let scan = "bssid / frequency / signal level / flags / ssid\n00:11\t2412\t-50\t[WPA2-PSK-CCMP][ESS]\tHome\n00:12\t2412\t-80\t[ESS]\tFree\n00:13\t5180\t-60\t[WPA2-PSK]\t\n";
        let v = parse_scan_results(scan, Some("Home"));
        assert_eq!(v.len(), 2);
        assert_eq!(
            (v[0].signal, v[0].secure, v[0].connected),
            (100, true, true)
        );
        assert_eq!((v[1].signal, v[1].secure), (40, false));
        assert_eq!(
            parse_wpa_status("wpa_state=COMPLETED\nssid=Home\n").as_deref(),
            Some("Home")
        );
        assert_eq!(parse_wpa_status("wpa_state=SCANNING\nssid=Home\n"), None);
        assert_eq!(parse_backlight("75\n"), Some(75));
        assert_eq!(parse_backlight("brightness: 30\n"), Some(30));
        assert_eq!(backlight_percent("10666", "21333"), Some(49));
        assert_eq!(backlight_percent("1", "0"), None);
        assert_eq!(
            parse_bluetoothctl("Controller x\n\tPowered: no\n"),
            Some(false)
        );
        assert_eq!(parse_bluetoothctl(""), None);
    }

    #[test]
    fn hypr_monitors_and_rules() {
        let j = r#"[{"name":"eDP-1","description":"BOE 0x1","width":1920,"height":1080,"refreshRate":60.003,"scale":1.25,"availableModes":["1920x1080@60.00Hz","1280x720@60.00Hz"]}]"#;
        let m = parse_hypr_monitors(j);
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].width, m[0].scale, m[0].modes.len()), (1920, 1.25, 2));
        assert_eq!(parse_mode("1280x720@59.94Hz"), Some((1280, 720, 59.94)));
        assert_eq!(
            monitor_rule("eDP-1", 1920, 1080, 60.0, 1.5).as_deref(),
            Some("eDP-1,1920x1080@60.00,auto,1.5")
        );
        assert_eq!(monitor_rule("eDP-1;exec x", 1, 1, 60.0, 1.0), None);
        assert_eq!(monitor_rule("eDP-1", 1, 1, 60.0, 9.0), None);
        assert!(parse_hypr_monitors("nope").is_empty());
    }

    fn controls(bsd: bool) -> (Arc<FakeHost>, Controls, Arc<Mutex<Vec<String>>>) {
        let h = Arc::new(FakeHost::new(bsd));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (l1, l2) = (log.clone(), log.clone());
        let c = Controls {
            host: h.clone(),
            stdin: Arc::new(move |argv, input| {
                l1.lock()
                    .unwrap()
                    .push(format!("{} <{}", argv.join(" "), input.trim()));
                Some(String::new())
            }),
            admin: Arc::new(move |req, pw| {
                l2.lock().unwrap().push(format!(
                    "admin {} {:?} pw={pw}",
                    req.action.name(),
                    req.args
                ));
                Some(if pw.is_empty() {
                    AdminReply::NeedsAuth
                } else {
                    AdminReply::Ok
                })
            }),
            wlan: "wlan0".into(),
        };
        (h, c, log)
    }

    #[test]
    fn linux_reads_and_writes() {
        let (h, c, log) = controls(false);
        h.command("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.64\n")
            .command("nmcli radio wifi", "enabled\n")
            .command("wpctl set-volume @DEFAULT_AUDIO_SINK@ 30%", "")
            .file("/sys/class/backlight/intel/brightness", "50\n")
            .file("/sys/class/backlight/intel/max_brightness", "200\n");
        assert_eq!(c.volume().map(|v| v.percent), Some(64));
        assert_eq!(c.wifi_enabled(), Some(true));
        assert_eq!(c.brightness(), Some(25));
        assert_eq!(c.set_volume(30), Ok(()));
        assert_eq!(c.set_muted(true), Err(ControlError::Failed));
        let _ = c.set_brightness(50);
        assert!(
            h.ran()
                .iter()
                .any(|r| r.ends_with("SetBrightness ssu backlight intel 100"))
        );
        assert_eq!(c.wifi_join("Home", Some("password1"), ""), Ok(()));
        let l = log.lock().unwrap().clone();
        assert!(l[0].contains(
            "connect Home passwd-file /dev/stdin <802-11-wireless-security.psk:password1"
        ));
        assert!(
            !h.ran().iter().any(|r| r.contains("password1")),
            "secret never in argv"
        );
        assert_eq!(
            c.wifi_join("Bad\nName", None, ""),
            Err(ControlError::Invalid)
        );
    }

    /// FreeBSD join: wol-admind only rewrites wpa_supplicant.conf; the answer waits for the network
    /// (wrong password = WrongPassword, not a silent success).
    #[test]
    fn freebsd_join_waits_for_the_network() {
        const NETS: &str = "network id / ssid / bssid / flags\n0\tHome\tany\t[TEMP-DISABLED]\n1\tCafe\tany\t[TEMP-DISABLED]\n";
        let (h, c, _) = controls(true);
        h.command("wpa_cli -i wlan0 status", "wpa_state=SCANNING\n")
            .command("wpa_cli -i wlan0 list_networks", NETS);
        assert_eq!(
            c.wifi_join("Home", Some("password1"), "secret"),
            Err(ControlError::WrongPassword)
        );
        assert_eq!(
            c.wifi_join("Cafe", None, "secret"),
            Err(ControlError::Failed),
            "open network refused: not a password problem"
        );
        h.command(
            "wpa_cli -i wlan0 status",
            "wpa_state=COMPLETED\nssid=Home\n",
        );
        assert_eq!(c.wifi_join("Home", Some("password1"), "secret"), Ok(()));
        let (_, c, _) = controls(true);
        assert_eq!(
            c.wifi_join("Home", Some("password1"), "secret"),
            Ok(()),
            "wpa_supplicant unreadable: trust the reconfigure"
        );
    }

    #[test]
    fn freebsd_admin_actions() {
        let (h, c, log) = controls(true);
        h.command(
            "mixer -o vol.volume vol.mute",
            "vol.volume=0.50:0.50\nvol.mute=0\n",
        )
        .command("backlight -q", "80\n");
        assert_eq!(c.volume().map(|v| v.percent), Some(50));
        assert_eq!(c.brightness(), Some(80));
        assert_eq!(
            c.wifi_join("Home", Some("password1"), ""),
            Err(ControlError::NeedsAuth)
        );
        assert_eq!(c.wifi_join("Home", Some("password1"), "secret"), Ok(()));
        assert_eq!(c.set_wifi(false, "secret"), Ok(()));
        assert_eq!(c.set_output("pcm1", "secret"), Ok(()));
        assert_eq!(c.set_output("pcmX", "secret"), Err(ControlError::Invalid));
        assert_eq!(
            c.add_user("dinah", "Dinah", false, "n3w-passw0rd", "secret"),
            Ok(())
        );
        let l = log.lock().unwrap().clone();
        assert_eq!(
            l[1],
            r#"admin wifi-join ["wlan0", "Home", "password1"] pw=secret"#
        );
        assert!(l[2].starts_with("admin wifi-power [\"wlan0\", \"off\"]"));
        assert!(l.iter().any(|x| x.starts_with("admin user-add")));
        assert_eq!(c.pkg_install("steam", "secret"), Ok(()));
        assert_eq!(c.pkg_install("-rf", "secret"), Err(ControlError::Invalid));
    }

    #[test]
    fn restore_point_and_revoke_go_to_wol_admind() {
        let (_, c, log) = controls(true);
        assert_eq!(c.boot_environment("wol-before-update-1", "secret"), Ok(()));
        assert_eq!(
            c.boot_environment("a;b", "secret"),
            Err(ControlError::Invalid)
        );
        c.revoke();
        let l = log.lock().unwrap().clone();
        assert_eq!(
            l,
            [
                r#"admin boot-environment ["wol-before-update-1"] pw=secret"#,
                "admin revoke [] pw="
            ]
        );
    }

    /// Settings sends any allow-listed request through one call; invalid ones never leave.
    #[test]
    fn any_admin_request_goes_through_one_call() {
        let (_, c, log) = controls(true);
        let req = |action, args: &[&str]| AdminRequest {
            action,
            args: sv(args),
        };
        assert_eq!(
            c.admin_request(&req(AdminAction::Hostname, &["julias-laptop"]), "secret"),
            Ok(())
        );
        assert_eq!(
            c.admin_request(&req(AdminAction::Firewall, &["on"]), ""),
            Err(ControlError::NeedsAuth)
        );
        assert_eq!(
            c.admin_request(&req(AdminAction::Hostname, &["-rf /"]), "secret"),
            Err(ControlError::Invalid)
        );
        assert_eq!(
            log.lock().unwrap().clone(),
            [
                r#"admin hostname ["julias-laptop"] pw=secret"#,
                r#"admin firewall ["on"] pw="#
            ]
        );
        let mut down = Controls::new(Arc::new(FakeHost::new(true)));
        down.admin = Arc::new(|_, _| None);
        assert_eq!(
            down.admin_request(&req(AdminAction::Ntp, &["on"]), "pw"),
            Err(ControlError::Unavailable)
        );
    }

    #[test]
    fn settings_reads() {
        let (h, c, _) = controls(true);
        let all = |c: &Controls| {
            let t = (c.network_time(), c.remote_login(), c.firewall());
            (t, c.hostname(), c.lid())
        };
        assert_eq!(all(&c), ((Some(false), None, None), None, None));
        h.command("service ntpd enabled", "")
            .command("sysrc -n sshd_enable", "YES\n")
            .command("sysrc -n pf_enable", "NO\n")
            .string("kern.hostname", "wolos")
            .string("hw.acpi.lid_switch_state", "S3");
        assert_eq!(
            all(&c),
            (
                (Some(true), Some(true), Some(false)),
                Some("wolos".into()),
                Some(crate::power::Lid::Sleep)
            )
        );
    }

    #[test]
    fn stdin_runner_and_missing_admind() {
        assert_eq!(
            run_with_input(&["cat"], "hi", std::time::Duration::from_secs(2)).as_deref(),
            Some("hi")
        );
        let req = AdminRequest {
            action: AdminAction::Revoke,
            args: vec![],
        };
        assert_eq!(
            admin_request(std::path::Path::new("/nonexistent/sock"), &req, ""),
            None
        );
    }
}
