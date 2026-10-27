//! Control Center extras: battery, Now Playing (MPRIS via playerctl), Bluetooth devices, keyboard
//! backlight, Low Power Mode, accessibility (Hyprland options), Night Shift probe, default route.
//! Same shape as the parent module: pure parsers (fixture-tested) + fixed argv per platform.
use super::{ControlError, Controls, backlight_percent};
use crate::power::policy::Mode;
use shared::admin::AdminAction;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Charge {
    Discharging,
    Charging,
    /// On power, not charging (full or held by a charge limit).
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BatteryState {
    pub percent: u8,
    pub charge: Charge,
    /// To empty while discharging, to full while charging; None when unknown.
    pub minutes: Option<u32>,
}

/// MPRIS player state (`playerctl metadata`).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Media {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Seconds.
    pub position: u32,
    pub length: Option<u32>,
    /// `mpris:artUrl` (file:// or http(s)://).
    pub art: Option<String>,
    /// MPRIS player name (`spotify`, `firefox`), for its app icon.
    #[serde(default)]
    pub player: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BtDevice {
    pub addr: String,
    pub name: String,
    pub connected: bool,
}

/// Hyprland-backed accessibility shortcuts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct A11y {
    pub reduce_motion: bool,
    pub zoom: bool,
}

/// Cursor zoom factor for Accessibility > Zoom.
pub const ZOOM_FACTOR: &str = "2";
/// Night Shift counts as on below this colour temperature (K).
const NIGHT_BELOW_K: u32 = 6000;

/// playerctl `--format`: one tab-separated line.
pub const PLAYERCTL_FORMAT: &str = "{{status}}\t{{position}}\t{{mpris:length}}\t{{xesam:title}}\t{{xesam:artist}}\t{{mpris:artUrl}}\t{{playerName}}";

/// powerd(8) flags for Low Power Mode and for the WolOS default (`image/rcconf.sh`).
pub const POWERD_LOW: &str = "-a minimum -b minimum";
pub const POWERD_AUTO: &str = "-a hiadaptive";

/// FreeBSD `hw.acpi.battery.{life,time,state}`: life -1 = no battery; state bit 1 discharging, 2 charging.
pub fn battery_bsd(life: i64, time: i64, state: i64) -> Option<BatteryState> {
    if !(0..=100).contains(&life) {
        return None;
    }
    let charge = if state & 2 != 0 {
        Charge::Charging
    } else if state & 1 != 0 {
        Charge::Discharging
    } else {
        Charge::Full
    };
    Some(BatteryState {
        percent: life as u8,
        charge,
        minutes: (charge == Charge::Discharging && time > 0).then_some(time as u32),
    })
}

/// Linux power_supply: capacity, status, energy (or charge) now/full and power (or current) now.
pub fn battery_sysfs(
    capacity: &str,
    status: &str,
    now: Option<u64>,
    full: Option<u64>,
    rate: Option<u64>,
) -> Option<BatteryState> {
    let percent = capacity.trim().parse::<u8>().ok()?.min(100);
    let charge = match status.trim() {
        "Charging" => Charge::Charging,
        "Discharging" => Charge::Discharging,
        _ => Charge::Full,
    };
    let minutes = match (charge, now, full, rate) {
        (_, _, _, None | Some(0)) => None,
        (Charge::Discharging, Some(n), _, Some(r)) => Some(n * 60 / r),
        (Charge::Charging, Some(n), Some(f), Some(r)) => Some(f.saturating_sub(n) * 60 / r),
        _ => None,
    };
    Some(BatteryState {
        percent,
        charge,
        minutes: minutes.map(|m| m.min(u32::MAX as u64) as u32),
    })
}

/// One [`PLAYERCTL_FORMAT`] line. None when no player has a title.
pub fn parse_playerctl(text: &str) -> Option<Media> {
    let line = text.lines().next()?;
    let f: Vec<&str> = line.split('\t').collect();
    let get = |i: usize| f.get(i).map(|s| s.trim()).unwrap_or("");
    let title = get(3);
    if title.is_empty() || get(0).is_empty() {
        return None;
    }
    let secs = |s: &str| s.parse::<u64>().ok().map(|us| (us / 1_000_000) as u32);
    Some(Media {
        title: title.into(),
        artist: get(4).into(),
        playing: get(0) == "Playing",
        position: secs(get(1)).unwrap_or(0),
        length: secs(get(2)).filter(|l| *l > 0),
        art: Some(get(5)).filter(|a| !a.is_empty()).map(String::from),
        player: get(6)
            .split(".instance")
            .next()
            .filter(|p| !p.is_empty())
            .map(String::from),
    })
}

/// `bluetoothctl devices [Paired|Connected]`: "Device AA:BB:CC:DD:EE:FF Name".
pub fn parse_bt_devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut w = l.trim().splitn(3, ' ');
            (w.next()? == "Device").then_some(())?;
            let addr = w.next()?;
            is_bt_addr(addr).then(|| (addr.to_string(), w.next().unwrap_or(addr).to_string()))
        })
        .collect()
}

pub fn is_bt_addr(s: &str) -> bool {
    s.len() == 17
        && s.split(':').count() == 6
        && s.split(':')
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `hyprctl -j getoption <name>`: the "int" or "float" value.
pub fn parse_hypr_option(json: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.get("int")
        .and_then(|x| x.as_f64())
        .or_else(|| v.get("float").and_then(|x| x.as_f64()))
}

/// `/etc/rc.conf` -> Low Power Mode on (powerd at minimum); None without powerd.
pub fn parse_powerd(rc_conf: &str) -> Option<bool> {
    let p = crate::system::pairs(rc_conf, '=');
    let val = |k: &str| p.get(k).map(|v| v.trim().trim_matches('"').to_string());
    (val("powerd_enable")?.eq_ignore_ascii_case("yes"))
        .then(|| val("powerd_flags").is_some_and(|f| f.contains("-a minimum")))
}

/// `route -n get default`: "interface: em0".
pub fn parse_route_iface(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("interface:"))
        .map(|i| i.trim().to_string())
        .filter(|i| !i.is_empty())
}

fn read_u64(c: &Controls, path: &str) -> Option<u64> {
    c.host.read_text(path)?.trim().parse().ok()
}

impl Controls {
    // ---- Battery ----

    pub fn battery(&self) -> Option<BatteryState> {
        let h = &self.host;
        if self.bsd() {
            return battery_bsd(
                h.sysctl_i64("hw.acpi.battery.life")?,
                h.sysctl_i64("hw.acpi.battery.time").unwrap_or(-1),
                h.sysctl_i64("hw.acpi.battery.state").unwrap_or(0),
            );
        }
        let dir = h
            .list_dir("/sys/class/power_supply")
            .into_iter()
            .find(|d| {
                h.read_text(&format!("{d}/type")).as_deref().map(str::trim) == Some("Battery")
            })?;
        let f = |n: &str| read_u64(self, &format!("{dir}/{n}"));
        let (now, full, rate) = match f("energy_now") {
            Some(n) => (Some(n), f("energy_full"), f("power_now")),
            None => (f("charge_now"), f("charge_full"), f("current_now")),
        };
        battery_sysfs(
            &h.read_text(&format!("{dir}/capacity"))?,
            &h.read_text(&format!("{dir}/status")).unwrap_or_default(),
            now,
            full,
            rate,
        )
    }

    // ---- Now Playing ----

    pub fn now_playing(&self) -> Option<Media> {
        self.run(&["playerctl", "metadata", "--format", PLAYERCTL_FORMAT])
            .and_then(|s| parse_playerctl(&s))
    }

    /// "play-pause", "next" or "previous".
    pub fn media(&self, verb: &str) -> Result<(), ControlError> {
        if !["play-pause", "next", "previous"].contains(&verb) {
            return Err(ControlError::Invalid);
        }
        self.ok(self.run(&["playerctl", verb]))
    }

    // ---- Bluetooth devices ----

    /// Paired devices (BlueZ). FreeBSD has no pairing backend yet: empty.
    pub fn bt_devices(&self) -> Vec<BtDevice> {
        if self.bsd() {
            return Vec::new();
        }
        let Some(paired) = self.run(&["bluetoothctl", "devices", "Paired"]) else {
            return Vec::new();
        };
        let connected: Vec<String> = self
            .run(&["bluetoothctl", "devices", "Connected"])
            .map(|s| parse_bt_devices(&s).into_iter().map(|d| d.0).collect())
            .unwrap_or_default();
        parse_bt_devices(&paired)
            .into_iter()
            .map(|(addr, name)| BtDevice {
                connected: connected.contains(&addr),
                addr,
                name,
            })
            .collect()
    }

    pub fn bt_connect(&self, addr: &str, on: bool) -> Result<(), ControlError> {
        if !is_bt_addr(addr) {
            return Err(ControlError::Invalid);
        }
        if self.bsd() {
            return Err(ControlError::Unavailable);
        }
        let verb = if on { "connect" } else { "disconnect" };
        match self.host.run_slow(&["bluetoothctl", verb, addr]) {
            Some(o) if o.contains("Failed") || o.contains("not available") => {
                Err(ControlError::Failed)
            }
            Some(_) => Ok(()),
            None => Err(ControlError::Failed),
        }
    }

    // ---- Keyboard backlight ----

    fn kbd_led(&self) -> Option<(String, u64)> {
        if self.bsd() {
            return None;
        }
        let dir = self
            .host
            .list_dir("/sys/class/leds")
            .into_iter()
            .find(|d| d.contains("kbd_backlight"))?;
        let max = read_u64(self, &format!("{dir}/max_brightness")).filter(|m| *m > 0)?;
        Some((dir, max))
    }

    /// Percent; None without a keyboard backlight.
    pub fn keyboard_brightness(&self) -> Option<u8> {
        let (dir, max) = self.kbd_led()?;
        backlight_percent(
            &self.host.read_text(&format!("{dir}/brightness"))?,
            &max.to_string(),
        )
    }

    /// logind `SetBrightness leds` (no root).
    pub fn set_keyboard_brightness(&self, percent: u8) -> Result<(), ControlError> {
        let (dir, max) = self.kbd_led().ok_or(ControlError::Unavailable)?;
        let dev = dir.rsplit('/').next().unwrap_or("");
        let raw = (max * percent.min(100) as u64).div_ceil(100).to_string();
        self.ok(self.run(&[
            "busctl",
            "call",
            "org.freedesktop.login1",
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.login1.Session",
            "SetBrightness",
            "ssu",
            "leds",
            dev,
            &raw,
        ]))
    }

    // ---- Low Power Mode ----

    /// Linux: power-profiles-daemon. FreeBSD: the Speed Shift preference (EPP), else powerd flags
    /// (rc.conf; rc.conf.d/powerd overrides powerd_enable). None: nothing to steer, hide the switch.
    pub fn low_power(&self) -> Option<bool> {
        if self.bsd() {
            return self.power_mode().map(|m| m == Mode::Low);
        }
        self.run(&["powerprofilesctl", "get"])
            .map(|s| s.trim() == "power-saver")
    }

    /// FreeBSD power mode (`power::policy`): the saved mode, when something steers the CPU (Speed
    /// Shift, or powerd enabled: rc.conf.d/powerd overrides rc.conf). Without a saved mode, powerd
    /// flags at minimum read as Low (systems set up before the policy). None: hide the switch.
    pub fn power_mode(&self) -> Option<Mode> {
        if !self.bsd() {
            return self
                .low_power()
                .map(|l| if l { Mode::Low } else { Mode::Auto });
        }
        let epp = self.host.sysctl_i64(&crate::power::epp_key(0)).is_some();
        let rc = self.host.read_text("/etc/rc.conf").unwrap_or_default();
        let rcd = self
            .host
            .read_text(&format!("{}/powerd", crate::hardware::RC_CONF_DIR))
            .unwrap_or_default();
        // `pairs` keeps the first value: rc.conf.d/powerd (sourced after rc.conf) goes first.
        let powerd = parse_powerd(&format!("{rcd}\n{rc}"));
        if !epp && powerd.is_none() {
            return None;
        }
        match self.host.read_text(crate::power::policy::STATE_FILE) {
            Some(t) => Some(crate::power::policy::State::parse(Some(&t)).mode),
            None if !epp && powerd == Some(true) => Some(Mode::Low),
            None => Some(Mode::Auto),
        }
    }

    pub fn set_low_power(&self, on: bool, pw: &str) -> Result<(), ControlError> {
        if self.bsd() {
            return self.set_power_mode(if on { Mode::Low } else { Mode::Auto }, pw);
        }
        let p = if on { "power-saver" } else { "balanced" };
        self.ok(self.run(&["powerprofilesctl", "set", p]))
    }

    /// wol-admind `power-mode` (no password: like brightness, any local user may change it).
    pub fn set_power_mode(&self, m: Mode, pw: &str) -> Result<(), ControlError> {
        if !self.bsd() {
            return self.set_low_power(m == Mode::Low, pw);
        }
        self.admin(AdminAction::PowerMode, &[m.name()], pw)
    }

    // ---- Accessibility (Hyprland, both platforms) ----

    pub fn accessibility(&self) -> Option<A11y> {
        let opt = |name: &str| {
            self.run(&["hyprctl", "-j", "getoption", name])
                .and_then(|s| parse_hypr_option(&s))
        };
        Some(A11y {
            reduce_motion: opt("animations:enabled")? == 0.0,
            zoom: opt("cursor:zoom_factor")? > 1.0,
        })
    }

    fn keyword(&self, name: &str, value: &str) -> Result<(), ControlError> {
        match self.run(&["hyprctl", "keyword", name, value]) {
            Some(o) if o.trim() == "ok" => Ok(()),
            _ => Err(ControlError::Failed),
        }
    }

    pub fn set_reduce_motion(&self, on: bool) -> Result<(), ControlError> {
        self.keyword("animations:enabled", if on { "0" } else { "1" })
    }

    pub fn set_zoom(&self, on: bool) -> Result<(), ControlError> {
        self.keyword("cursor:zoom_factor", if on { ZOOM_FACTOR } else { "1" })
    }

    // ---- Night Shift probe ----

    /// hyprsunset's current temperature: None when hyprsunset is not running (tile hidden).
    pub fn night_shift(&self) -> Option<bool> {
        let t: u32 = self
            .run(&["hyprctl", "hyprsunset", "temperature"])?
            .trim()
            .parse()
            .ok()?;
        Some(t < NIGHT_BELOW_K)
    }

    // ---- Network ----

    /// FreeBSD default-route interface (no sysfs): "em0", "wlan0"; None offline.
    pub fn default_iface(&self) -> Option<String> {
        self.run(&["route", "-n", "get", "default"])
            .and_then(|s| parse_route_iface(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;
    use shared::admin::AdminReply;
    use std::sync::{Arc, Mutex};

    fn controls(bsd: bool) -> (Arc<FakeHost>, Controls, Arc<Mutex<Vec<String>>>) {
        let h = Arc::new(FakeHost::new(bsd));
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        let mut c = Controls::new(h.clone());
        c.admin = Arc::new(move |req, pw| {
            l.lock()
                .unwrap()
                .push(format!("{} {:?}", req.action.name(), req.args));
            Some(if pw.is_empty() {
                AdminReply::NeedsAuth
            } else {
                AdminReply::Ok
            })
        });
        (h, c, log)
    }

    #[test]
    fn battery_table() {
        use Charge::*;
        for (life, time, state, want) in [
            (82, 191, 1, Some((82, Discharging, Some(191)))),
            (40, -1, 2, Some((40, Charging, None))),
            (100, -1, 0, Some((100, Full, None))),
            (-1, -1, 7, None),
        ] {
            assert_eq!(
                battery_bsd(life, time, state).map(|b| (b.percent, b.charge, b.minutes)),
                want,
                "{life} {time} {state}"
            );
        }
        let b = battery_sysfs(
            "50\n",
            "Discharging\n",
            Some(30_000_000),
            None,
            Some(10_000_000),
        );
        assert_eq!(
            b.map(|b| (b.charge, b.minutes)),
            Some((Discharging, Some(180)))
        );
        let b = battery_sysfs("50", "Charging", Some(30), Some(50), Some(10));
        assert_eq!(b.map(|b| b.minutes), Some(Some(120)));
        let b = battery_sysfs("80", "Not charging", Some(30), Some(50), Some(0));
        assert_eq!(b.map(|b| (b.charge, b.minutes)), Some((Full, None)));
        assert_eq!(battery_sysfs("x", "Full", None, None, None), None);
    }

    #[test]
    fn battery_reads_per_platform() {
        let (h, c, _) = controls(true);
        assert_eq!(c.battery(), None, "desktop: no battery sysctls");
        h.int("hw.acpi.battery.life", 64)
            .int("hw.acpi.battery.time", 125)
            .int("hw.acpi.battery.state", 1);
        assert_eq!(
            c.battery(),
            Some(BatteryState {
                percent: 64,
                charge: Charge::Discharging,
                minutes: Some(125)
            })
        );
        let (h, c, _) = controls(false);
        h.file("/sys/class/power_supply/AC/type", "Mains\n");
        assert_eq!(c.battery(), None);
        h.file("/sys/class/power_supply/BAT0/type", "Battery\n")
            .file("/sys/class/power_supply/BAT0/capacity", "71\n")
            .file("/sys/class/power_supply/BAT0/status", "Charging\n")
            .file("/sys/class/power_supply/BAT0/charge_now", "2000\n")
            .file("/sys/class/power_supply/BAT0/charge_full", "4000\n")
            .file("/sys/class/power_supply/BAT0/current_now", "1000\n");
        assert_eq!(
            c.battery().map(|b| (b.percent, b.minutes)),
            Some((71, Some(120)))
        );
    }

    #[test]
    fn playerctl_lines() {
        let m = parse_playerctl(
            "Playing\t61000000\t240000000\tWhite Rabbit\tJefferson Airplane\tfile:///tmp/a.png\n",
        )
        .unwrap();
        assert_eq!(
            (m.playing, m.position, m.length, m.art.as_deref()),
            (true, 61, Some(240), Some("file:///tmp/a.png"))
        );
        let m = parse_playerctl("Paused\t\t\tTalk\t\t\n").unwrap();
        assert_eq!((m.playing, m.length, m.art), (false, None, None));
        assert_eq!(parse_playerctl("Stopped\t\t\t\t\t\n"), None);
        assert_eq!(parse_playerctl(""), None);
    }

    /// Now Playing without cover art shows the player's app icon: the MPRIS player name, without
    /// the per-process `.instanceN` suffix browsers add.
    #[test]
    fn playerctl_names_the_player() {
        for (tail, want) in [
            ("\tspotify", Some("spotify")),
            ("\tchromium.instance4321", Some("chromium")),
            ("\t", None),
            ("", None),
        ] {
            let m = parse_playerctl(&format!("Playing\t0\t0\tSong\tBand\t{tail}\n")).unwrap();
            assert_eq!(m.player.as_deref(), want, "{tail:?}");
        }
        assert!(PLAYERCTL_FORMAT.ends_with("\t{{playerName}}"));
    }

    #[test]
    fn media_verbs() {
        let (h, c, _) = controls(false);
        h.command("playerctl next", "");
        assert_eq!(c.media("next"), Ok(()));
        assert_eq!(
            c.media("play-pause"),
            Err(ControlError::Failed),
            "no player"
        );
        assert_eq!(c.media("open /etc/passwd"), Err(ControlError::Invalid));
        h.command(
            &format!("playerctl metadata --format {PLAYERCTL_FORMAT}"),
            "Playing\t0\t0\tSong\tBand\t\n",
        );
        assert_eq!(c.now_playing().map(|m| m.title), Some("Song".into()));
    }

    #[test]
    fn bluetooth_devices() {
        let (h, c, _) = controls(false);
        h.command(
            "bluetoothctl devices Paired",
            "Device AA:BB:CC:DD:EE:01 AirPods Pro\nDevice AA:BB:CC:DD:EE:02 MX Keys\njunk\n",
        )
        .command(
            "bluetoothctl devices Connected",
            "Device AA:BB:CC:DD:EE:02 MX Keys\n",
        )
        .slow(
            "bluetoothctl connect AA:BB:CC:DD:EE:01",
            "Attempting to connect\nConnection successful\n",
        );
        let d = c.bt_devices();
        assert_eq!(
            d.iter()
                .map(|d| (d.name.as_str(), d.connected))
                .collect::<Vec<_>>(),
            [("AirPods Pro", false), ("MX Keys", true)]
        );
        assert_eq!(c.bt_connect("AA:BB:CC:DD:EE:01", true), Ok(()));
        assert_eq!(
            c.bt_connect("AA:BB:CC:DD:EE:02", false),
            Err(ControlError::Failed)
        );
        assert_eq!(c.bt_connect("AA:BB; rm", true), Err(ControlError::Invalid));
        let (_, bsd, _) = controls(true);
        assert!(bsd.bt_devices().is_empty());
        assert_eq!(
            bsd.bt_connect("AA:BB:CC:DD:EE:01", true),
            Err(ControlError::Unavailable)
        );
    }

    #[test]
    fn keyboard_backlight() {
        let (h, c, _) = controls(false);
        assert_eq!(c.keyboard_brightness(), None);
        h.file("/sys/class/leds/input3::capslock/brightness", "0\n")
            .file("/sys/class/leds/tpacpi::kbd_backlight/brightness", "1\n")
            .file(
                "/sys/class/leds/tpacpi::kbd_backlight/max_brightness",
                "2\n",
            );
        assert_eq!(c.keyboard_brightness(), Some(50));
        let _ = c.set_keyboard_brightness(100);
        assert!(
            h.ran()
                .iter()
                .any(|r| r.ends_with("SetBrightness ssu leds tpacpi::kbd_backlight 2")),
            "{:?}",
            h.ran()
        );
        let (_, bsd, _) = controls(true);
        assert_eq!(bsd.keyboard_brightness(), None);
        assert_eq!(
            bsd.set_keyboard_brightness(50),
            Err(ControlError::Unavailable)
        );
    }

    #[test]
    fn low_power_mode() {
        assert_eq!(
            parse_powerd("powerd_enable=\"YES\"\npowerd_flags=\"-a minimum -b minimum\"\n"),
            Some(true)
        );
        assert_eq!(
            parse_powerd("powerd_enable=\"YES\"\npowerd_flags=\"-a hiadaptive\"\n"),
            Some(false)
        );
        assert_eq!(parse_powerd("sshd_enable=YES\n"), None);
        let (h, c, log) = controls(true);
        h.file(
            "/etc/rc.conf",
            "powerd_enable=\"YES\"\npowerd_flags=\"-a hiadaptive\"\n",
        );
        assert_eq!(c.low_power(), Some(false));
        assert_eq!(c.set_low_power(true, ""), Err(ControlError::NeedsAuth));
        assert_eq!(c.set_low_power(true, "pw"), Ok(()));
        assert_eq!(log.lock().unwrap()[1], r#"power-mode ["low"]"#);
        let (h, c, _) = controls(false);
        h.command("powerprofilesctl get", "power-saver\n")
            .command("powerprofilesctl set balanced", "");
        assert_eq!(c.low_power(), Some(true));
        assert_eq!(c.set_low_power(false, ""), Ok(()));
    }

    /// Speed Shift: the EPP sysctl is the truth; powerd may be off (rc.conf.d/powerd from the
    /// hardware module overrides rc.conf). Neither: None, the switch hides.
    #[test]
    fn low_power_reads_epp_first_then_powerd() {
        use crate::power::policy::{Mode, STATE_FILE};
        let (h, c, log) = controls(true);
        h.file(
            "/etc/rc.conf",
            "powerd_enable=\"YES\"\npowerd_flags=\"-a hiadaptive\"\n",
        )
        .file("/etc/rc.conf.d/powerd", "powerd_enable=\"NO\"\n");
        assert_eq!(c.low_power(), None, "no EPP, powerd off");
        // The preference follows the power source (75 on battery): the saved mode is the truth.
        h.int("dev.hwpstate_intel.0.epp", 85);
        assert_eq!(c.power_mode(), Some(Mode::Auto), "no saved mode yet");
        h.file(STATE_FILE, "{\"mode\":\"low\"}\n");
        assert_eq!(
            (c.power_mode(), c.low_power()),
            (Some(Mode::Low), Some(true))
        );
        h.file(STATE_FILE, "{\"mode\":\"high\"}\n");
        assert_eq!(
            (c.power_mode(), c.low_power()),
            (Some(Mode::High), Some(false))
        );
        assert_eq!(c.set_power_mode(Mode::High, "pw"), Ok(()));
        assert_eq!(
            log.lock().unwrap().last().unwrap(),
            r#"power-mode ["high"]"#
        );
        let (h, c, _) = controls(true);
        h.file(
            "/etc/rc.conf",
            "powerd_enable=\"NO\"\npowerd_flags=\"-a minimum -b minimum\"\n",
        )
        .file("/etc/rc.conf.d/powerd", "powerd_enable=\"YES\"\n");
        assert_eq!(
            c.low_power(),
            Some(true),
            "rc.conf.d/powerd turns powerd on; no saved mode: the flags tell"
        );
        h.file(STATE_FILE, "{\"mode\":\"auto\"}\n");
        assert_eq!(c.low_power(), Some(false));
        assert_eq!(controls(true).1.low_power(), None, "no rc.conf");
    }

    #[test]
    fn accessibility_options() {
        let (h, c, _) = controls(true);
        assert_eq!(c.accessibility(), None, "no Hyprland");
        h.command(
            "hyprctl -j getoption animations:enabled",
            r#"{"option":"animations:enabled","int":0,"set":true}"#,
        )
        .command(
            "hyprctl -j getoption cursor:zoom_factor",
            r#"{"option":"cursor:zoom_factor","float":1.000000,"set":false}"#,
        )
        .command("hyprctl keyword cursor:zoom_factor 2", "ok")
        .command("hyprctl keyword animations:enabled 1", "ok\n");
        assert_eq!(
            c.accessibility(),
            Some(A11y {
                reduce_motion: true,
                zoom: false
            })
        );
        assert_eq!(c.set_zoom(true), Ok(()));
        assert_eq!(c.set_reduce_motion(false), Ok(()));
        assert_eq!(c.set_reduce_motion(true), Err(ControlError::Failed));
    }

    #[test]
    fn night_shift_and_route_probes() {
        let (h, c, _) = controls(true);
        assert_eq!(c.night_shift(), None, "hyprsunset not running");
        h.command("hyprctl hyprsunset temperature", "4500\n");
        assert_eq!(c.night_shift(), Some(true));
        h.command("hyprctl hyprsunset temperature", "6500");
        assert_eq!(c.night_shift(), Some(false));
        h.command("hyprctl hyprsunset temperature", "invalid command");
        assert_eq!(c.night_shift(), None);
        assert_eq!(c.default_iface(), None);
        h.command(
            "route -n get default",
            "   route to: default\ndestination: default\n  interface: wlan0\n      flags: <UP,GATEWAY,DONE,STATIC>\n",
        );
        assert_eq!(c.default_iface().as_deref(), Some("wlan0"));
    }
}
