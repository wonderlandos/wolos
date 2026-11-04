// SPDX-License-Identifier: BSD-2-Clause
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
    /// What the network asks for (QA-NET-9: an 802.1X network needs more than a password).
    #[serde(default)]
    pub security: crate::wifi::Security,
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
    /// "WxH@R" strings as the compositor lists them.
    pub modes: Vec<String>,
    /// The output this one mirrors (`hyprctl -j monitors all` "mirrorOf"; "none" or absent =
    /// None). Read from the compositor, so a reload or an unplug never leaves a stale mirror (QA-SET-47).
    #[serde(default)]
    pub mirror_of: Option<String>,
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
    /// Wi-Fi join: no network of that name in range (QA-NET-8).
    NotFound,
    /// The network needs something WolOS cannot do yet (802.1X Enterprise Wi-Fi, QA-NET-9).
    Unsupported,
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
        let ssid = crate::wifi::unescape(ssid);
        if ssid.is_empty() {
            continue;
        }
        let security = crate::wifi::security_kind(sec);
        let n = WifiNet {
            ssid,
            signal: sig.trim().parse::<u8>().unwrap_or(0).min(100),
            secure: matches!(
                security,
                crate::wifi::Security::Psk | crate::wifi::Security::Enterprise
            ),
            connected: used.trim() == "*",
            security,
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

// ---- The compositor ----

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
                mirror_of: m
                    .get("mirrorOf")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty() && *s != "none")
                    .map(String::from),
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

/// Compositor `monitor=` value: "eDP-1,1920x1080@60.00,auto,1.25".
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

/// The dimmest backlight a change sets: 0 % would turn some panels off (one rule for the setter
/// and the shell's optimistic snapshot, QA-SET-53).
pub const BRIGHTNESS_MIN: u8 = 1;

/// How long a Wi-Fi join (login window, wpa_supplicant) waits for the network.
pub const JOIN_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// Platform reads and writes. `stdin` runs argv with input on stdin (secrets); `admin` sends one wol-admind request.
pub type StdinRunner = dyn Fn(&[&str], &str) -> Result<String, String> + Send + Sync;
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
            // nmcli waits up to 90 s for an activation (`--wait`, its default): a slower runner
            // cut joins that were about to work (QA-NET-8).
            stdin: Arc::new(|argv, input| {
                run_with_input_err(argv, input, std::time::Duration::from_secs(JOIN_SECS))
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

    /// NetworkManager radio switch. `None` when there is no Wi-Fi device (nmcli reports the
    /// radio "enabled" with no adapter, QA-NET-7) or NetworkManager does not answer.
    pub fn wifi_enabled(&self) -> Option<bool> {
        // Both reads side by side: `wol wifi` and the snapshot wait for the slower one only.
        let (devs, radio) = std::thread::scope(|sc| {
            let d = sc.spawn(|| self.run(&["nmcli", "-t", "-f", "TYPE", "dev"]));
            let r = self.run(&["nmcli", "radio", "wifi"]);
            (d.join().ok().flatten(), r)
        });
        if devs.is_some_and(|d| !d.lines().any(|l| l.trim() == "wifi")) {
            return None;
        }
        radio.map(|o| o.trim() == "enabled")
    }

    /// The scan. `--rescan auto`: NetworkManager scans first when its list is older than 30 s
    /// (QA-NET-16: `--rescan no` showed a hotspot switched on after the pane opened minutes late).
    pub fn wifi_networks(&self) -> Vec<WifiNet> {
        self.run(&crate::wifi::NM_LIST_ARGV)
            .map(|s| parse_nmcli_wifi(&s))
            .unwrap_or_default()
    }

    /// NetworkManager lets the person at the computer switch the radio (polkit): no password.
    /// No Wi-Fi device, or NetworkManager not answering: Unavailable (QA-PWR-21).
    pub fn set_wifi(&self, on: bool, _pw: &str) -> Result<(), ControlError> {
        let v = if on { "on" } else { "off" };
        if self.run(&["nmcli", "radio", "wifi", v]).is_some() {
            return Ok(());
        }
        match self.wifi_enabled() {
            None => Err(ControlError::Unavailable),
            Some(_) => Err(ControlError::Failed),
        }
    }

    /// Joins (slow: call off the UI thread). The password goes on stdin, never argv.
    pub fn wifi_join(
        &self,
        ssid: &str,
        password: Option<&str>,
        admin_pw: &str,
    ) -> Result<(), ControlError> {
        self.wifi_join_hidden(ssid, password, false, admin_pw)
    }

    /// [`Controls::wifi_join`]; `hidden`: the network does not announce its name, so
    /// NetworkManager asks for it by name (`802-11-wireless.hidden yes`, QA-NET-10).
    pub fn wifi_join_hidden(
        &self,
        ssid: &str,
        password: Option<&str>,
        hidden: bool,
        _admin_pw: &str,
    ) -> Result<(), ControlError> {
        use crate::wifi::{JoinFailure, Security};
        if crate::wifi::validate(ssid, password).is_some() {
            return Err(ControlError::Invalid);
        }
        let scan = self.run(&crate::wifi::NM_SCAN_ARGV).unwrap_or_default();
        // QA-NET-9: an 802.1X network takes a user name and password (or a certificate), never
        // a PSK: sending one only ever ended in "wrong password".
        if crate::wifi::nm_security(&scan, ssid) == Some(Security::Enterprise) {
            return Err(ControlError::Unsupported);
        }
        let out = match password {
            // NET2-1: a profile (made, or changed in place when one is saved), then
            // `connection up ... passwd-file /dev/stdin` (the secret on stdin).
            // `dev wifi connect ... passwd-file` is not valid nmcli.
            Some(p) => {
                let has = self
                    .run(&crate::wifi::NM_PROFILES_ARGV)
                    .is_some_and(|l| crate::wifi::nm_has_profile(&l, ssid));
                let km = crate::wifi::nm_key_mgmt(&scan, ssid);
                for step in crate::wifi::nm_profile_argvs(ssid, hidden, km, has) {
                    let step: Vec<&str> = step.iter().map(String::as_str).collect();
                    if self.host.run_slow(&step).is_none() {
                        return Err(self.wifi_cause(ControlError::Failed));
                    }
                }
                let up = crate::wifi::nm_up_argv(ssid);
                let up: Vec<&str> = up.iter().map(String::as_str).collect();
                let r = (self.stdin)(&up, &crate::wifi::nm_secret_input(p));
                if r.is_err() && !has {
                    // A wrong password leaves no profile behind.
                    let del = crate::wifi::nm_cleanup_argv(ssid);
                    let del: Vec<&str> = del.iter().map(String::as_str).collect();
                    let _ = self.host.run_slow(&del);
                }
                r
            }
            None => {
                let mut v = vec!["nmcli", "dev", "wifi", "connect", ssid];
                if hidden {
                    v.extend(["hidden", "yes"]);
                }
                (self.stdin)(&v, "")
            }
        };
        let Err(stderr) = out else {
            return Ok(());
        };
        // QA-NET-8: nmcli says why; only a refused secret is a wrong password.
        Err(
            match crate::wifi::classify_join_error(&stderr, password.is_some()) {
                JoinFailure::WrongPassword => ControlError::WrongPassword,
                JoinFailure::NoDevice => ControlError::Unavailable,
                JoinFailure::NotFound => ControlError::NotFound,
                JoinFailure::NoAddress => ControlError::Failed,
                JoinFailure::Other => self.wifi_cause(ControlError::Failed),
            },
        )
    }

    /// `e`, or Unavailable when there is no Wi-Fi device or NetworkManager does not answer: asked
    /// only after a failure, so a join that works costs no extra read.
    fn wifi_cause(&self, e: ControlError) -> ControlError {
        if self.wifi_enabled().is_none() {
            ControlError::Unavailable
        } else {
            e
        }
    }

    /// Leaves the network `iface` is on (QA-NET-17); the profile stays, so it joins again by
    /// itself next time. NetworkManager lets the person at the computer do this (polkit).
    pub fn wifi_disconnect(&self, iface: &str) -> Result<(), ControlError> {
        let a = crate::wifi::nm_disconnect_argv(iface).ok_or(ControlError::Invalid)?;
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        self.ok(self.host.run_slow(&a))
    }

    /// Wi-Fi hotspot (internet sharing, NET2-2): NetworkManager's shared mode on the Wi-Fi device
    /// (its own dnsmasq hands out addresses). The password goes on stdin. `None` turns it off.
    pub fn set_hotspot(&self, on: Option<(&str, &str)>) -> Result<(), ControlError> {
        use crate::wifi::{HOTSPOT_PROFILE, hotspot_argvs, nm_secret_input, nm_up_argv};
        let Some((ssid, password)) = on else {
            return self.ok(self.host.run_slow(&[
                "nmcli",
                "connection",
                "down",
                "id",
                HOTSPOT_PROFILE,
            ]));
        };
        if crate::wifi::validate(ssid, Some(password)).is_some() {
            return Err(ControlError::Invalid);
        }
        if self.wifi_enabled().is_none() {
            return Err(ControlError::Unavailable);
        }
        let has = self
            .run(&crate::wifi::NM_PROFILES_ARGV)
            .is_some_and(|l| crate::wifi::nm_has_profile(&l, HOTSPOT_PROFILE));
        for step in hotspot_argvs(ssid, has) {
            let step: Vec<&str> = step.iter().map(String::as_str).collect();
            if self.host.run_slow(&step).is_none() {
                return Err(ControlError::Failed);
            }
        }
        let up = nm_up_argv(HOTSPOT_PROFILE);
        let up: Vec<&str> = up.iter().map(String::as_str).collect();
        (self.stdin)(&up, &nm_secret_input(password))
            .map(|_| ())
            .map_err(|e| match crate::wifi::classify_join_error(&e, false) {
                crate::wifi::JoinFailure::NoDevice => ControlError::Unavailable,
                _ => ControlError::Failed,
            })
    }

    /// Is the hotspot on: its profile active (`nmcli -t -f NAME connection show --active`).
    pub fn hotspot(&self) -> Option<bool> {
        self.wifi_enabled()?;
        let active = self.run(&crate::wifi::NM_ACTIVE_ARGV)?;
        Some(
            active
                .lines()
                .any(|l| crate::wifi::unescape(l) == crate::wifi::HOTSPOT_PROFILE),
        )
    }

    // ---- Audio ----

    /// PipeWire default sink (wpctl).
    pub fn volume(&self) -> Option<Volume> {
        self.run(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"])
            .and_then(|s| parse_wpctl_volume(&s))
    }

    /// `wpctl set-volume N%`: N counts from the hardware maximum, capped at 100 (no boost here).
    /// This is the path of the volume keys, `wol volume` and a machine with no sound service. The
    /// sliders (Settings, Control Center) go through `wol_audio` instead, which knows each
    /// device's 0 dB (`wol_core::audio::Action::OutputVolume`), so they never use this.
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
        let p = percent.clamp(BRIGHTNESS_MIN, 100);
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
    /// None without an adapter (bluetoothctl is not run: it waits for org.bluez for ever, QA-DEV2-3).
    pub fn bluetooth(&self) -> Option<bool> {
        if crate::bluetooth::adapters(&*self.host).is_empty() {
            return None;
        }
        self.run(&["bluetoothctl", "show"])
            .and_then(|s| parse_bluetoothctl(&s))
    }

    pub fn set_bluetooth(&self, on: bool, _pw: &str) -> Result<(), ControlError> {
        if crate::bluetooth::adapters(&*self.host).is_empty() {
            return Err(ControlError::Unavailable);
        }
        let v = if on { "on" } else { "off" };
        self.ok(self.run(&["bluetoothctl", "power", v]))
    }

    // ---- Displays (the compositor) ----

    pub fn monitors(&self) -> Vec<Monitor> {
        self.run(&["hyprctl", "-j", "monitors", "all"])
            .map(|s| parse_hypr_monitors(&s))
            .unwrap_or_default()
    }

    /// One `monitor =` rule, live. Then X11 apps follow a new scale: `wol-x11-env` re-merges
    /// Xft.dpi and Xcursor.size and re-exports GDK_SCALE and Steam's UI scale (audit X1-2). Its
    /// result is ignored: without an X server or a user bus it skips, and the next `exec` on
    /// hyprctl reload catches up.
    pub fn set_monitor(&self, rule: &str) -> Result<(), ControlError> {
        let r = self.ok(self.run(&["hyprctl", "keyword", "monitor", rule]));
        if r.is_ok() {
            let _ = self.run(&[crate::hyprprefs::X11_ENV]);
        }
        r
    }

    /// Reduce Motion from the Control Center: the same saved preference Settings > Accessibility
    /// writes (`reduce_motion`, applied live by [`crate::hyprprefs::apply_desktop`]), so the tile
    /// and the pane are one switch and the choice survives the next login (QA-SET-17).
    pub fn set_reduce_motion_saved(
        &self,
        home: &std::path::Path,
        on: bool,
    ) -> Result<(), ControlError> {
        let mut d = crate::hyprprefs::load_desktop(home);
        let same = d.reduce_motion == on;
        d.reduce_motion = on;
        crate::hyprprefs::apply_desktop(&*self.host, home, &d).map_err(|_| ControlError::Failed)?;
        if same {
            // Saved already; make the live keyword agree (an older tile only set that).
            let _ = self.set_reduce_motion(on);
        }
        Ok(())
    }

    /// hyprsunset IPC (`exec-once = hyprsunset`). Night Light has one state: after hyprsunset
    /// took the change, the toggle flag `wol toggle nightlight` lists follows it (QA-SET-46).
    pub fn set_night_shift(&self, on: bool) -> Result<(), ControlError> {
        self.set_night_shift_in(on, &crate::journal::state_home())
    }

    /// [`Controls::set_night_shift`] with the state home given.
    pub fn set_night_shift_in(
        &self,
        on: bool,
        state_home: &std::path::Path,
    ) -> Result<(), ControlError> {
        self.hyprsunset(on)?;
        let _ = crate::toggles::set_flag(
            crate::toggles::Toggle::NightLight,
            std::path::Path::new(""),
            state_home,
            on,
        );
        Ok(())
    }

    fn hyprsunset(&self, on: bool) -> Result<(), ControlError> {
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

/// How long a Wi-Fi join may take: nmcli's own activation wait (90 s) and a margin.
pub const JOIN_SECS: u64 = 100;

/// Run argv with `input` on stdin: stdout when it exits 0 within `timeout`, else its stderr
/// (what [`crate::wifi::classify_join_error`] reads), or "timed out" when it was killed.
pub fn run_with_input_err(
    argv: &[&str],
    input: &str,
    timeout: std::time::Duration,
) -> Result<String, String> {
    crate::system::run_input_err(argv, input, timeout)
}

/// Run argv with `input` on stdin; stdout when it exits 0 within `timeout`.
pub fn run_with_input(argv: &[&str], input: &str, timeout: std::time::Duration) -> Option<String> {
    let (file, args) = argv.split_first()?;
    let mut cmd = std::process::Command::new(file);
    cmd.args(args);
    crate::system::run_input(cmd, input, timeout)
}

/// wol-admind's sentence from the last failed reply of each action in this process (the
/// optional message of `failed <message>`, `shared::admin::failed_line`), taken once by the
/// surface that shows it ([`take_failure`]). Set and cleared by [`admin_request`].
static FAILURES: std::sync::Mutex<Vec<(AdminAction, String)>> = std::sync::Mutex::new(Vec::new());

/// Records (or, with None, clears) the sentence for `action`: [`admin_request`] after every reply,
/// and the shell after a wopkg run that reported wol-admind's words.
pub fn note_failure(action: AdminAction, message: Option<String>) {
    let mut f = FAILURES.lock().unwrap_or_else(|p| p.into_inner());
    f.retain(|(a, _)| *a != action);
    f.extend(message.map(|m| (action, m)));
}

/// The sentence wol-admind gave with the last failed `action` request (Update Now's restore
/// point refusal, say); None when it gave none or it was taken already.
pub fn take_failure(action: AdminAction) -> Option<String> {
    let mut f = FAILURES.lock().unwrap_or_else(|p| p.into_inner());
    let i = f.iter().position(|(a, _)| *a == action)?;
    Some(f.remove(i).1)
}

/// One wol-admind round trip. None when the daemon is not running. Long actions (pkg) may take minutes.
pub fn admin_request(
    sock: &std::path::Path,
    req: &AdminRequest,
    password: &str,
) -> Option<AdminReply> {
    use std::io::{Read, Write};
    // wol-admind greets first (ADR 0033, audit S4-7): answer its hello before the request.
    let mut s = shared::jobs::connect(sock).ok()?;
    // As long as wol-admind may take (S4-4: every planned step at its own bound, plus margin).
    let wait = std::time::Duration::from_secs(crate::admin::reply_secs(req).into());
    let _ = s.set_read_timeout(Some(wait));
    s.write_all(&shared::admin::encode(req, password.as_bytes()))
        .ok()?;
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut line = String::new();
    s.take(256).read_to_string(&mut line).ok()?;
    note_failure(req.action, shared::admin::failure_message(&line));
    Some(AdminReply::parse(&line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;
    use std::sync::Mutex;

    /// QA-NET-19: nmcli's terse output escapes `\\` as well as `:`; an SSID with a backslash
    /// reads as typed.
    #[test]
    fn an_ssid_with_a_backslash_reads_as_typed() {
        let v = parse_nmcli_wifi("*:60:WPA2:Cafe\\\\Bar\n :40:WPA2:A\\\\\\:B\n");
        assert_eq!(v[0].ssid, "Cafe\\Bar");
        assert_eq!(v[1].ssid, "A\\:B");
        assert_eq!(
            crate::netinfo::parse_nmcli_connections("Cafe\\\\Bar:802-11-wireless\n"),
            ["Cafe\\Bar"]
        );
    }

    /// QA-NET-16: Settings and Control Center ask NetworkManager for a fresh list when theirs is
    /// old (`--rescan auto`), never only what it happens to remember.
    #[test]
    fn wifi_lists_ask_for_a_fresh_scan() {
        let (h, c, _) = controls();
        h.command(
            "nmcli -t -f IN-USE,SIGNAL,SECURITY,SSID dev wifi list --rescan auto",
            "*:60:WPA2:Home\n",
        );
        assert_eq!(c.wifi_networks().len(), 1);
        assert!(
            h.ran().iter().all(|l| !l.contains("--rescan no")),
            "{:?}",
            h.ran()
        );
    }

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
    fn audio_volume_lines_parse() {
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
    fn backlight_and_other_lines_parse() {
        assert_eq!(backlight_percent("10666", "21333"), Some(49));
        assert_eq!(backlight_percent("1", "0"), None);
        assert_eq!(
            parse_bluetoothctl("Controller x\n\tPowered: no\n"),
            Some(false)
        );
        assert_eq!(parse_bluetoothctl(""), None);
    }

    /// QA-SET-47: the mirror comes from the compositor ("mirrorOf"), not the page's memory.
    #[test]
    fn mirror_of_from_the_compositor() {
        let m = parse_hypr_monitors(
            r#"[{"name":"HDMI-A-1","mirrorOf":"eDP-1"},{"name":"eDP-1","mirrorOf":"none"},{"name":"DP-1"}]"#,
        );
        assert_eq!(m[0].mirror_of.as_deref(), Some("eDP-1"));
        assert_eq!(
            (m[1].mirror_of.as_deref(), m[2].mirror_of.as_deref()),
            (None, None)
        );
    }

    #[test]
    fn hypr_monitors_and_rules() {
        let j = r#"[{"name":"eDP-1","description":"BOE 0x1","width":1920,"height":1080,"refreshRate":60.003,"scale":1.25,
"availableModes":["1920x1080@60.00Hz","1280x720@60.00Hz"]}]"#;
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
        // A Wi-Fi device with the radio on (tests that need none say so).
        h.command("nmcli -t -f TYPE dev", "wifi\n")
            .command("nmcli radio wifi", "enabled\n");
        let log = Arc::new(Mutex::new(Vec::new()));
        let (l1, l2) = (log.clone(), log.clone());
        let c = Controls {
            host: h.clone(),
            stdin: Arc::new(move |argv, input| {
                l1.lock()
                    .unwrap()
                    .push(format!("{} <{}", argv.join(" "), input.trim()));
                Ok(String::new())
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

    const ADD_HOME: &str =
        "nmcli connection add type wifi con-name Home ifname * ssid Home wifi-sec.key-mgmt wpa-psk";

    const MODIFY_HOME: &str = "nmcli connection modify id Home wifi-sec.key-mgmt wpa-psk";

    /// NET2-1: the password join never uses `dev wifi connect ... passwd-file` (nmcli rc 2); a
    /// saved Wi-Fi profile of that name is changed in place (not deleted), a profile that cannot
    /// be made is a failure, not "wrong password", and a failed bring-up is "wrong password".
    #[test]
    fn wifi_join_makes_profile_then_brings_it_up_with_the_secret_on_stdin() {
        let (h, c, log) = controls();
        h.slow(MODIFY_HOME, "").command(
            "nmcli -t -f NAME,TYPE connection show",
            "Wired connection 1:802-3-ethernet\nHome:802-11-wireless\n",
        );
        assert_eq!(c.wifi_join("Home", Some("password1"), ""), Ok(()));
        assert_eq!(
            log.lock().unwrap().clone(),
            [
                "nmcli connection up id Home passwd-file /dev/stdin <802-11-wireless-security.psk:password1"
            ]
        );
        // WPA3-only network: sae.
        let (h3, c3, _) = controls();
        h3.slow(
            "nmcli connection add type wifi con-name Home ifname * ssid Home wifi-sec.key-mgmt sae",
            "",
        )
        .command("nmcli -t -f SECURITY,SSID dev wifi list", "WPA3:Home\n");
        assert_eq!(c3.wifi_join("Home", Some("password1"), ""), Ok(()));
        // The profile cannot be changed: nothing is brought up.
        let (h2, c2, log2) = controls();
        h2.command(
            "nmcli -t -f NAME,TYPE connection show",
            "Home:802-11-wireless\n",
        );
        assert_eq!(
            c2.wifi_join("Home", Some("password1"), ""),
            Err(ControlError::Failed)
        );
        assert!(log2.lock().unwrap().is_empty(), "nothing brought up");
        // The bring-up fails: wrong password.
        let h4 = Arc::new(FakeHost::linux());
        h4.slow(
            "nmcli connection add type wifi con-name Home ifname * ssid Home wifi-sec.key-mgmt wpa-psk",
            "",
        );
        let c4 = Controls {
            host: h4,
            stdin: Arc::new(|_, _| Err(SECRETS.into())),
            admin: Arc::new(|_, _| None),
            wlan: "wlan0".into(),
        };
        assert_eq!(
            c4.wifi_join("Home", Some("password1"), ""),
            Err(ControlError::WrongPassword)
        );
    }

    const SECRETS: &str = "Error: Connection activation failed: Secrets were required, but not provided.\nHint: use 'journalctl -xe \
        NM_CONNECTION=1f0e NM_DEVICE=wlan0' to get more details.\n";

    /// QA-NET-8: nmcli's stderr names the cause; only a refused secret reads as a wrong password.
    /// Fixtures are nmcli 1.58's own words.
    #[test]
    fn nmcli_stderr_classified() {
        use crate::wifi::{JoinFailure::*, classify_join_error};
        for (err, secret, want) in [
            (SECRETS, true, WrongPassword),
            (
                "Error: Connection activation failed: The 802.1X supplicant disconnected\n",
                true,
                WrongPassword,
            ),
            ("Error: No Wi-Fi device found.\n", true, NoDevice),
            (
                "Error: Connection activation failed: No suitable device found for this connection (device lo not available because device is \
                    strictly unmanaged).\n",
                true,
                NoDevice,
            ),
            ("Error: NetworkManager is not running.\n", false, NoDevice),
            (
                "Error: No network with SSID 'Foo' found.\n",
                false,
                NotFound,
            ),
            (
                "Error: Connection activation failed: The Wi-Fi network could not be found\n",
                true,
                NotFound,
            ),
            (
                "Error: Connection activation failed: IP configuration could not be reserved (no available address, timeout, etc.).\n",
                true,
                NoAddress,
            ),
            ("Error: Timeout expired (90 seconds)\n", true, NoAddress),
            ("timed out", true, NoAddress),
            (SECRETS, false, Other),
            ("", true, Other),
        ] {
            assert_eq!(classify_join_error(err, secret), want, "{err}");
        }
        // Through Controls: each cause has its own error.
        for (err, want) in [
            (SECRETS, ControlError::WrongPassword),
            ("Error: No Wi-Fi device found.\n", ControlError::Unavailable),
            (
                "Error: Connection activation failed: The Wi-Fi network could not be found\n",
                ControlError::NotFound,
            ),
            (
                "Error: Timeout expired (90 seconds)\n",
                ControlError::Failed,
            ),
        ] {
            let h = Arc::new(FakeHost::linux());
            h.slow(
                "nmcli connection add type wifi con-name Home ifname * ssid Home wifi-sec.key-mgmt wpa-psk",
                "",
            );
            let c = Controls {
                host: h,
                stdin: Arc::new(move |_, _| Err(err.into())),
                admin: Arc::new(|_, _| None),
                wlan: "wlan0".into(),
            };
            assert_eq!(
                c.wifi_join("Home", Some("password1"), ""),
                Err(want),
                "{err}"
            );
        }
    }

    /// QA-NET-9: an 802.1X network is never sent a PSK; its kind travels with the scan.
    #[test]
    fn parse_eap_kind() {
        use crate::wifi::Security;
        let v =
            parse_nmcli_wifi(" :70:WPA2 802.1X:Campus\n :60:WPA2:Home\n :50::Cafe\n :40:OWE:Lib\n");
        let kinds: Vec<_> = v
            .iter()
            .map(|n| (n.ssid.as_str(), n.security, n.secure))
            .collect();
        assert_eq!(
            kinds,
            [
                ("Campus", Security::Enterprise, true),
                ("Home", Security::Psk, true),
                ("Cafe", Security::Open, false),
                ("Lib", Security::Owe, false),
            ]
        );
        let (h, c, log) = controls();
        h.command(
            "nmcli -t -f SECURITY,SSID dev wifi list",
            "WPA2 802.1X:Campus\n",
        );
        assert_eq!(
            c.wifi_join("Campus", Some("password1"), ""),
            Err(ControlError::Unsupported)
        );
        assert!(
            log.lock().unwrap().is_empty() && h.ran().iter().all(|r| !r.contains("connection"))
        );
    }

    /// QA-NET-10: a hidden network is asked for by name, with or without a password.
    #[test]
    fn hidden_join_says_hidden_yes() {
        let (h, c, log) = controls();
        h.slow(
            "nmcli connection add type wifi con-name Attic ifname * ssid Attic wifi-sec.key-mgmt wpa-psk 802-11-wireless.hidden yes",
            "",
        );
        assert_eq!(
            c.wifi_join_hidden("Attic", Some("password1"), true, ""),
            Ok(())
        );
        assert!(log.lock().unwrap()[0].starts_with("nmcli connection up id Attic passwd-file"));
        assert_eq!(c.wifi_join_hidden("Open Attic", None, true, ""), Ok(()));
        assert_eq!(
            log.lock().unwrap()[1],
            "nmcli dev wifi connect Open Attic hidden yes <"
        );
    }

    /// QA-NET-17: Disconnect leaves the network and keeps the profile; NET2-2: the hotspot
    /// profile is made without the secret in argv, brought up with it on stdin.
    #[test]
    fn disconnect_and_hotspot() {
        let (h, c, log) = controls();
        h.slow("nmcli dev disconnect wlan0", "");
        assert_eq!(c.wifi_disconnect("wlan0"), Ok(()));
        assert_eq!(c.wifi_disconnect("-x"), Err(ControlError::Invalid));
        h.command("nmcli -t -f TYPE dev", "wifi\n")
            .command("nmcli radio wifi", "enabled\n")
            .slow(
                "nmcli connection add type wifi con-name WolOS Hotspot ifname * ssid My Hot 802-11-wireless.mode ap 802-11-wireless.band bg \
                    ipv4.method shared ipv6.method ignore wifi-sec.key-mgmt wpa-psk wifi-sec.psk-flags 0 connection.autoconnect no",
                "",
            );
        assert_eq!(c.set_hotspot(Some(("My Hot", "hot12345"))), Ok(()));
        assert_eq!(
            log.lock().unwrap().last().unwrap(),
            "nmcli connection up id WolOS Hotspot passwd-file /dev/stdin <802-11-wireless-security.psk:hot12345"
        );
        assert!(h.ran().iter().all(|r| !r.contains("hot12345")));
        h.command(
            "nmcli -t -f NAME connection show --active",
            "WolOS Hotspot\nHome\n",
        );
        assert_eq!(c.hotspot(), Some(true));
        h.slow("nmcli connection down id WolOS Hotspot", "");
        assert_eq!(c.set_hotspot(None), Ok(()));
        // No Wi-Fi device: nothing is made.
        let (h2, c2, _) = controls();
        h2.command("nmcli -t -f TYPE dev", "ethernet\n");
        assert_eq!(
            c2.set_hotspot(Some(("My Hot", "hot12345"))),
            Err(ControlError::Unavailable)
        );
        assert_eq!(c2.hotspot(), None);
    }

    #[test]
    fn wifi_enabled_none_without_device() {
        let (h, c, _log) = controls();
        h.command("nmcli radio wifi", "enabled\n")
            .command("nmcli -t -f TYPE dev", "ethernet\nloopback\n");
        assert_eq!(c.wifi_enabled(), None);
    }

    #[test]
    fn linux_reads_and_writes() {
        let (h, c, log) = controls();
        h.command("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.64\n")
            .command("nmcli radio wifi", "enabled\n")
            .command("nmcli -t -f TYPE dev", "ethernet\nwifi\nloopback\n")
            .command("wpctl set-volume @DEFAULT_AUDIO_SINK@ 30%", "")
            .slow(ADD_HOME, "")
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
        assert!(
            l[0].contains("connection up id Home passwd-file /dev/stdin <802-11-wireless-security.psk:password1"),
            "{l:?}"
        );
        assert!(!l[0].contains("dev wifi connect"), "{l:?}");

        assert!(
            !h.ran().iter().any(|r| r.contains("password1")),
            "secret never in argv"
        );
        assert_eq!(
            c.wifi_join("Bad\nName", None, ""),
            Err(ControlError::Invalid)
        );
    }

    /// QA-DEV2-3/4: no adapter, no bluetoothctl; the answer is immediate.
    #[test]
    fn bluetooth_without_adapter_runs_nothing() {
        let (h, c, _) = controls();
        h.command("bluetoothctl show", "Controller x\n\tPowered: yes\n");
        assert_eq!(c.bluetooth(), None);
        assert_eq!(c.set_bluetooth(true, ""), Err(ControlError::Unavailable));
        assert!(h.ran().iter().all(|r| !r.contains("bluetoothctl")));
    }

    /// Radio, Bluetooth and the output need no administrator (NetworkManager, BlueZ, PipeWire);
    /// accounts and packages still go through wol-admind.
    #[test]
    fn local_changes_and_admin_actions() {
        let (h, c, log) = controls();
        h.file("/sys/class/bluetooth/hci0/address", "00:1A:7D:DA:71:13\n");
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
    fn settings_reads_answer_none_then_values() {
        let (h, c, _) = controls();
        assert_eq!((c.network_time(), c.lid()), (None, None));
        h.command("timedatectl show -p NTP --value", "yes\n")
            .file("/proc/acpi/button/lid/LID0/state", "state: open\n");
        assert_eq!(
            (c.network_time(), c.lid()),
            (Some(true), Some(crate::power::Lid::Sleep))
        );
    }

    /// QA-SET-46: the tile set hyprsunset and left the flag `wol toggle nightlight` reads, so the
    /// hotkey needed two presses after a click. One change, one state.
    #[test]
    fn set_night_shift_sets_toggle_state() {
        use crate::toggles::{Toggle, is_on};
        let (h, c, _) = controls();
        h.command("hyprctl hyprsunset temperature 4500", "ok\n")
            .command("hyprctl hyprsunset identity", "ok\n");
        let d = std::env::temp_dir().join(format!("wol-night-shift-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let none = std::path::Path::new("");
        let on = |d: &std::path::Path| is_on(Toggle::NightLight, none, d, |p| p.exists());
        assert!(!on(&d));
        c.set_night_shift_in(true, &d).unwrap();
        assert!(on(&d), "on: the flag says on");
        c.set_night_shift_in(false, &d).unwrap();
        assert!(!on(&d), "off: the flag says off");
        // hyprsunset refuses: nothing changed, the flag stays as it was
        let (_, c, _) = controls();
        assert!(c.set_night_shift_in(true, &d).is_err());
        assert!(!on(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// QA-SET-17: the Control Center tile only flipped the live keyword, Settings > Accessibility
    /// saved the preference; one of them was lost at the next login.
    #[test]
    fn reduce_motion_persists() {
        let (h, c, _) = controls();
        let home = std::env::temp_dir().join(format!("wol-reduce-motion-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(!crate::hyprprefs::load_desktop(&home).reduce_motion);
        c.set_reduce_motion_saved(&home, true).unwrap();
        assert!(crate::hyprprefs::load_desktop(&home).reduce_motion, "saved");
        assert!(
            h.ran().iter().any(|r| r.contains("animations")),
            "and applied live: {:?}",
            h.ran()
        );
        c.set_reduce_motion_saved(&home, false).unwrap();
        assert!(!crate::hyprprefs::load_desktop(&home).reduce_motion);
        let _ = std::fs::remove_dir_all(&home);
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

    /// Audit S4-7 / ADR 0033: wol-admind greets first; the client answers the hello and only
    /// then sends the request, which reaches the daemon whole.
    #[test]
    fn admin_request_answers_wol_admind_s_hello() {
        use std::io::{Read, Write};
        let dir = std::env::temp_dir().join(format!("wol-ctl-admin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("admin.sock");
        let l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let req = AdminRequest {
            action: AdminAction::Revoke,
            args: vec![],
        };
        let want = shared::admin::encode(&req, b"");
        let daemon = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            shared::jobs::greet(&c).unwrap();
            let mut got = Vec::new();
            c.read_to_end(&mut got).unwrap();
            c.write_all(b"ok\n").unwrap();
            got
        });
        assert_eq!(admin_request(&path, &req, ""), Some(AdminReply::Ok));
        assert_eq!(daemon.join().unwrap(), want);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed reply's sentence (Update Now's refusal) is kept for the page that asked, once;
    /// a plain `failed` or the next request of the same action clears it.
    #[test]
    fn a_failed_reply_leaves_its_sentence_for_the_page() {
        use std::io::{Read, Write};
        let dir = std::env::temp_dir().join(format!("wol-ctl-failed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("admin.sock");
        let l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let replies = [
            shared::admin::failed_line("No restore point could be kept."),
            "failed".to_string(),
        ];
        let daemon = std::thread::spawn(move || {
            for r in replies {
                let (mut c, _) = l.accept().unwrap();
                shared::jobs::greet(&c).unwrap();
                let mut got = Vec::new();
                c.read_to_end(&mut got).unwrap();
                c.write_all(format!("{r}\n").as_bytes()).unwrap();
            }
        });
        let up = AdminRequest {
            action: AdminAction::PkgUpgrade,
            args: vec![],
        };
        assert_eq!(admin_request(&path, &up, ""), Some(AdminReply::Failed));
        assert_eq!(
            take_failure(AdminAction::Ntp),
            None,
            "only the action that failed"
        );
        assert_eq!(
            take_failure(AdminAction::PkgUpgrade).as_deref(),
            Some("No restore point could be kept.")
        );
        assert_eq!(take_failure(AdminAction::PkgUpgrade), None, "taken once");
        note_failure(AdminAction::PkgUpgrade, Some("stale".into()));
        assert_eq!(admin_request(&path, &up, ""), Some(AdminReply::Failed));
        assert_eq!(
            take_failure(AdminAction::PkgUpgrade),
            None,
            "a plain failure clears it"
        );
        daemon.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A display change reaches X11 apps too (Settings > Displays, Control Center).
    #[test]
    fn a_monitor_change_refreshes_the_x11_scale() {
        let (h, c, _) = controls();
        let rule = "DP-1,2560x1440@60,0x0,2";
        h.command(&format!("hyprctl keyword monitor {rule}"), "ok");
        c.set_monitor(rule).unwrap();
        let ran = h.ran();
        let at = |l: &str| ran.iter().position(|r| r == l);
        let kw = at(&format!("hyprctl keyword monitor {rule}")).expect("keyword sent");
        assert!(at("wol-x11-env").is_some_and(|x| x > kw), "{ran:?}");
        // A refused rule changes nothing, so the X11 step does not run.
        let (h, c, _) = controls();
        assert!(c.set_monitor("bad").is_err());
        assert!(!h.ran().iter().any(|r| r == "wol-x11-env"), "{:?}", h.ran());
    }
}
