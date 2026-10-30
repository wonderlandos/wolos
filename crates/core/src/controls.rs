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

/// How long a Wi-Fi join (login window, wpa_supplicant) waits for the network.
pub const JOIN_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// Platform reads and writes. `stdin` runs argv with input on stdin (secrets); `admin` sends one wol-admind request.
pub type StdinRunner = dyn Fn(&[&str], &str) -> Option<String> + Send + Sync;
pub type AdminSender = dyn Fn(&AdminRequest, &str) -> Option<AdminReply> + Send + Sync;

pub struct Controls {
    pub host: Arc<dyn Host>,
    pub stdin: Arc<StdinRunner>,
    pub admin: Arc<AdminSender>,
    /// Wireless interface name (shown in Settings; NetworkManager needs none).
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

    // ---- Flatpak ----

    /// Turns one recorded Flatpak permission on or off (`perm`), or back to what the app asked
    /// for (`None`). A user install changes as the user; a system one goes through wol-admind.
    pub fn flatpak_override(
        &self,
        scope: crate::flatpak::Scope,
        app_id: &str,
        perm: Option<&str>,
        allow: bool,
        pw: &str,
    ) -> Result<(), ControlError> {
        use crate::flatpak::{Perm, Scope, is_app_id, reset_argv};
        let parsed = perm.map(Perm::parse);
        if !is_app_id(app_id) || parsed.as_ref().is_some_and(Option::is_none) {
            return Err(ControlError::Invalid);
        }
        match scope {
            Scope::System => {
                let verdict = if allow { "allow" } else { "deny" };
                let args: Vec<&str> = match perm {
                    Some(p) => vec![app_id, p, verdict],
                    None => vec![app_id, "reset"],
                };
                self.admin(AdminAction::FlatpakOverride, &args, pw)
            }
            Scope::User => {
                let argv = match parsed.flatten() {
                    Some(p) => p.override_argv(Scope::User, app_id, allow),
                    None => reset_argv(Scope::User, app_id),
                };
                let a: Vec<&str> = argv.iter().map(String::as_str).collect();
                self.ok(self.run(&a))
            }
        }
    }

    // ---- Wi-Fi ----

    /// NetworkManager radio switch.
    pub fn wifi_enabled(&self) -> Option<bool> {
        self.run(&["nmcli", "radio", "wifi"])
            .map(|o| o.trim() == "enabled")
    }

    pub fn wifi_networks(&self) -> Vec<WifiNet> {
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

    /// NetworkManager lets the person at the computer switch the radio (polkit): no password.
    pub fn set_wifi(&self, on: bool, _pw: &str) -> Result<(), ControlError> {
        let v = if on { "on" } else { "off" };
        self.ok(self.run(&["nmcli", "radio", "wifi", v]))
    }

    /// Joins (slow: call off the UI thread). The password goes on stdin, never argv.
    pub fn wifi_join(
        &self,
        ssid: &str,
        password: Option<&str>,
        _admin_pw: &str,
    ) -> Result<(), ControlError> {
        if crate::wifi::validate(ssid, password).is_some() {
            return Err(ControlError::Invalid);
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

    // ---- Audio ----

    /// PipeWire default sink (wpctl).
    pub fn volume(&self) -> Option<Volume> {
        self.run(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"])
            .and_then(|s| parse_wpctl_volume(&s))
    }

    pub fn set_volume(&self, percent: u8) -> Result<(), ControlError> {
        let p = percent.min(100);
        self.ok(self.run(&[
            "wpctl",
            "set-volume",
            "@DEFAULT_AUDIO_SINK@",
            &format!("{p}%"),
        ]))
    }

    pub fn set_muted(&self, muted: bool) -> Result<(), ControlError> {
        let m = if muted { "1" } else { "0" };
        self.ok(self.run(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", m]))
    }

    /// Sinks through pipewire-pulse.
    pub fn outputs(&self) -> Vec<Output> {
        let def = self.run(&["pactl", "get-default-sink"]).unwrap_or_default();
        self.run(&["pactl", "list", "sinks"])
            .map(|s| parse_pactl_sinks(&s, &def))
            .unwrap_or_default()
    }

    pub fn set_output(&self, id: &str, _pw: &str) -> Result<(), ControlError> {
        if id.is_empty() || id.starts_with('-') {
            return Err(ControlError::Invalid);
        }
        self.ok(self.run(&["pactl", "set-default-sink", id]))
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

    /// No fork (sysfs).
    pub fn brightness(&self) -> Option<u8> {
        let (dir, max) = self.linux_backlight()?;
        backlight_percent(
            &self.host.read_text(&format!("{dir}/brightness"))?,
            &max.to_string(),
        )
    }

    /// logind `SetBrightness` (no root).
    pub fn set_brightness(&self, percent: u8) -> Result<(), ControlError> {
        let p = percent.clamp(1, 100);
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

    /// BlueZ adapter powered.
    pub fn bluetooth(&self) -> Option<bool> {
        self.run(&["bluetoothctl", "show"])
            .and_then(|s| parse_bluetoothctl(&s))
    }

    pub fn set_bluetooth(&self, on: bool, _pw: &str) -> Result<(), ControlError> {
        let v = if on { "on" } else { "off" };
        self.ok(self.run(&["bluetoothctl", "power", v]))
    }

    // ---- Displays (Hyprland) ----

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

    /// App Store: remove a catalog id or package name (slow; worker thread).
    pub fn pkg_remove(&self, target: &str, pw: &str) -> Result<(), ControlError> {
        self.admin(AdminAction::PkgRemove, &[target], pw)
    }

    pub fn pkg_upgrade(&self, pw: &str) -> Result<(), ControlError> {
        self.admin(AdminAction::PkgUpgrade, &[], pw)?;
        self.flatpak_update()
    }

    /// App Store: a Flathub app for this account (no password; the consent sheet asks).
    pub fn flatpak_install(&self, app_id: &str) -> Result<(), ControlError> {
        self.flatpak_run(app_id, crate::flatpak::install_argv)
    }

    pub fn flatpak_remove(&self, app_id: &str) -> Result<(), ControlError> {
        self.flatpak_run(app_id, crate::flatpak::remove_argv)
    }

    fn flatpak_run(&self, app_id: &str, f: fn(&str) -> Vec<String>) -> Result<(), ControlError> {
        if !crate::flatpak::is_app_id(app_id) {
            return Err(ControlError::Invalid);
        }
        let argv = f(app_id);
        let a: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.ok(self.host.run_slow(&a))
    }

    /// Software Update: this account's Flatpak apps too. Nothing installed = nothing to do.
    pub fn flatpak_update(&self) -> Result<(), ControlError> {
        let list = crate::flatpak::installed_argv();
        let l: Vec<&str> = list.iter().map(String::as_str).collect();
        let none = self
            .run(&l)
            .is_none_or(|t| crate::flatpak::parse_ids(&t).is_empty());
        if none {
            return Ok(());
        }
        let argv = crate::flatpak::update_argv();
        let a: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.ok(self.host.run_slow(&a))
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

    /// AirPlay Receiver (shairport-sync) on at boot (`airplay-receiver`).
    pub fn airplay_receiver(&self) -> Option<bool> {
        crate::sharing::airplay_receiver(&*self.host)
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
    use std::io::Write;
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
    crate::system::wait_output(child, timeout)
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
        let sinks = "Sink #43\n\tState: RUNNING\n\tName: mic_sink\n\tDescription: Mic\nSink #70\n\tName: alsa.out\n\tDescription: Built-in Audio\n";
        let o = parse_pactl_sinks(sinks, "alsa.out\n");
        assert_eq!(o.len(), 2);
        assert_eq!(
            (o[1].id.as_str(), o[1].name.as_str(), o[1].default),
            ("alsa.out", "Built-in Audio", true)
        );
    }

    #[test]
    fn wpa_and_misc_parsers() {
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

    fn controls() -> (Arc<FakeHost>, Controls, Arc<Mutex<Vec<String>>>) {
        let h = Arc::new(FakeHost::linux());
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
    fn flatpak_store_actions_run_as_the_user() {
        let (h, c, _) = controls();
        assert_eq!(c.flatpak_update(), Ok(()), "no flatpak apps: nothing to do");
        h.slow(
            "flatpak install --user --noninteractive -y flathub org.kde.krita",
            "",
        )
        .command(
            "flatpak list --app --columns=application",
            "org.kde.krita\n",
        )
        .slow("flatpak update --user --noninteractive -y", "");
        assert_eq!(c.flatpak_install("org.kde.krita"), Ok(()));
        assert_eq!(c.flatpak_remove("org.kde.krita"), Err(ControlError::Failed));
        assert_eq!(c.flatpak_install("-y"), Err(ControlError::Invalid));
        assert_eq!(c.flatpak_update(), Ok(()));
    }

    #[test]
    fn flatpak_overrides_per_scope() {
        use crate::flatpak::Scope;
        let (h, c, log) = controls();
        h.command(
            "flatpak override --user --unshare=network org.gimp.GIMP",
            "",
        )
        .command("flatpak override --user --reset org.gimp.GIMP", "");
        assert_eq!(
            c.flatpak_override(
                Scope::User,
                "org.gimp.GIMP",
                Some("shared=network"),
                false,
                ""
            ),
            Ok(())
        );
        assert_eq!(
            c.flatpak_override(Scope::User, "org.gimp.GIMP", None, true, ""),
            Ok(())
        );
        assert_eq!(
            c.flatpak_override(
                Scope::System,
                "org.gimp.GIMP",
                Some("devices=all"),
                true,
                "pw"
            ),
            Ok(())
        );
        assert_eq!(
            log.lock().unwrap().last().unwrap(),
            r#"admin flatpak-override ["org.gimp.GIMP", "devices=all", "allow"] pw=pw"#
        );
        for (id, perm) in [("gimp", None), ("org.gimp.GIMP", Some("--socket=x11"))] {
            assert_eq!(
                c.flatpak_override(Scope::User, id, perm, true, ""),
                Err(ControlError::Invalid)
            );
        }
    }

    #[test]
    fn linux_reads_and_writes() {
        let (h, c, log) = controls();
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

    /// Radio, Bluetooth and the output need no administrator (NetworkManager, BlueZ, PipeWire);
    /// accounts and packages still go through wol-admind.
    #[test]
    fn local_changes_and_admin_actions() {
        let (h, c, log) = controls();
        h.command("nmcli radio wifi off", "")
            .command("bluetoothctl power on", "Changing power on succeeded\n")
            .command("pactl set-default-sink alsa.out", "");
        assert_eq!(c.set_wifi(false, ""), Ok(()));
        assert_eq!(c.set_bluetooth(true, ""), Ok(()));
        assert_eq!(c.set_output("alsa.out", ""), Ok(()));
        assert_eq!(c.set_output("-x", ""), Err(ControlError::Invalid));
        assert_eq!(
            c.add_user("dinah", "Dinah", false, "n3w-passw0rd", ""),
            Err(ControlError::NeedsAuth)
        );
        assert_eq!(
            c.add_user("dinah", "Dinah", false, "n3w-passw0rd", "secret"),
            Ok(())
        );
        let l = log.lock().unwrap().clone();
        assert!(l.iter().all(|x| x.starts_with("admin user-add")), "{l:?}");
        assert_eq!(c.pkg_install("steam", "secret"), Ok(()));
        assert_eq!(c.pkg_install("-rf", "secret"), Err(ControlError::Invalid));
    }

    #[test]
    fn restore_point_and_revoke_go_to_wol_admind() {
        let (_, c, log) = controls();
        assert_eq!(c.boot_environment("wolos-b", "secret"), Ok(()));
        assert_eq!(
            c.boot_environment("a;b", "secret"),
            Err(ControlError::Invalid)
        );
        c.revoke();
        let l = log.lock().unwrap().clone();
        assert_eq!(
            l,
            [
                r#"admin boot-environment ["wolos-b"] pw=secret"#,
                "admin revoke [] pw="
            ]
        );
    }

    /// Settings sends any allow-listed request through one call; invalid ones never leave.
    #[test]
    fn any_admin_request_goes_through_one_call() {
        let (_, c, log) = controls();
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
        let mut down = Controls::new(Arc::new(FakeHost::linux()));
        down.admin = Arc::new(|_, _| None);
        assert_eq!(
            down.admin_request(&req(AdminAction::Ntp, &["on"]), "pw"),
            Err(ControlError::Unavailable)
        );
    }

    #[test]
    fn settings_reads() {
        let (h, c, _) = controls();
        assert_eq!((c.network_time(), c.lid()), (None, None));
        h.command("timedatectl show -p NTP --value", "yes\n")
            .file("/proc/acpi/button/lid/LID0/state", "state: open\n");
        assert_eq!(
            (c.network_time(), c.lid()),
            (Some(true), Some(crate::power::Lid::Sleep))
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
