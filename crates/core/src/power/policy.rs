//! Power policy: the power source picks the profile, Low Power / High Power Mode is the user's
//! override. Pure: facts from read-only probes + the saved state in, a plan of fixed argv out.
//! `wol power apply` (udev on AC changes, systemd at boot and resume) and wol-admind `power-mode`
//! run the same plan. The CPU and platform energy profile belong to power-profiles-daemon; the lid
//! action is systemd-logind's (wol-admind `lid`, [`super::LID_CONF`]).
use super::Lid;
use serde::{Deserialize, Serialize};

/// World-readable; wol-admind `power-mode` writes it, `wol power apply` reads it.
pub const STATE_FILE: &str = "/etc/wolos/power.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Follows the power source.
    #[default]
    Auto,
    /// Low Power Mode: efficiency first on AC and battery.
    Low,
    /// High Power Mode: speed first.
    High,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Auto, Mode::Low, Mode::High];
    pub fn name(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Low => "low",
            Mode::High => "high",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub mode: Mode,
}

impl State {
    /// Missing or broken file: Automatic.
    pub fn parse(text: Option<&str>) -> Self {
        text.and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default()
    }
    pub fn to_json(self) -> String {
        serde_json::to_string(&self).unwrap_or_default() + "\n"
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// Mains, or a desktop without a Mains power supply.
    #[default]
    Ac,
    Battery,
}

/// Who steers the CPU clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cpu {
    /// power-profiles-daemon: CPU energy preference (intel_pstate, amd-pstate-epp) and the ACPI
    /// platform profile.
    Epp,
    /// Nothing to steer (virtual machine, no power-profiles-daemon).
    #[default]
    Fixed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wlan {
    pub name: String,
    /// The driver supports 802.11 power save (`iw dev <if> get power_save` answers).
    pub power_save: bool,
}

/// What the probes found. A failing probe means the hardware or service is absent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub ncpu: u32,
    pub cpu: Cpu,
    /// power-profiles-daemon profile now (`balanced`).
    pub profile: Option<String>,
    pub source: Source,
    /// Battery capacity %, None without a battery.
    pub battery: Option<u8>,
    /// Lid action now; None without a lid.
    pub lid: Option<Lid>,
    /// Suspend to RAM (`mem` in /sys/power/state).
    pub sleep: bool,
    pub wlans: Vec<Wlan>,
}

pub const NCPU_ARGV: [&str; 1] = ["nproc"];
pub const PROFILE_ARGV: [&str; 2] = ["powerprofilesctl", "get"];
pub const SUPPLIES_ARGV: [&str; 2] = ["ls", "/sys/class/power_supply"];
pub const LID_ARGV: [&str; 2] = ["ls", "/proc/acpi/button/lid"];
pub const LID_CONF_ARGV: [&str; 2] = ["cat", super::LID_CONF];
pub const SLEEP_ARGV: [&str; 2] = ["cat", "/sys/power/state"];
pub const NET_ARGV: [&str; 2] = ["ls", "/sys/class/net"];

/// A sysfs entry name that may reach argv: no path separators, no options.
fn is_name(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && !s.starts_with(['-', '.'])
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
}

/// `probe`: fixed argv -> stdout, None when it failed (a missing file fails `cat` and `ls`).
pub fn facts(probe: &dyn Fn(&[&str]) -> Option<String>) -> Facts {
    let text = |argv: &[&str]| probe(argv).map(|s| s.trim().to_owned());
    let names = |argv: &[&str]| -> Vec<String> {
        text(argv)
            .unwrap_or_default()
            .split_whitespace()
            .filter(|n| is_name(n))
            .map(String::from)
            .collect()
    };
    let profile = text(&PROFILE_ARGV).filter(|p| !p.is_empty());
    let (mut mains, mut on_ac, mut battery) = (false, false, None);
    for n in names(&SUPPLIES_ARGV) {
        let f = |k: &str| text(&["cat", &format!("/sys/class/power_supply/{n}/{k}")]);
        match f("type").as_deref() {
            Some("Mains") => {
                mains = true;
                on_ac |= f("online").as_deref() == Some("1");
            }
            Some("Battery") if battery.is_none() && f("scope").as_deref() != Some("Device") => {
                battery = f("capacity")
                    .and_then(|c| c.parse::<u8>().ok())
                    .filter(|c| *c <= 100);
            }
            _ => {}
        }
    }
    let wlans = names(&NET_ARGV)
        .into_iter()
        .filter(|n| probe(&["ls", &format!("/sys/class/net/{n}/wireless")]).is_some())
        .map(|n| Wlan {
            power_save: probe(&["iw", "dev", &n, "get", "power_save"]).is_some(),
            name: n,
        })
        .collect();
    Facts {
        ncpu: text(&NCPU_ARGV)
            .and_then(|s| s.parse::<u32>().ok())
            .map_or(0, |n| n.min(4096)),
        cpu: if profile.is_some() {
            Cpu::Epp
        } else {
            Cpu::Fixed
        },
        profile,
        source: if mains && !on_ac && battery.is_some() {
            Source::Battery
        } else {
            Source::Ac
        },
        battery,
        lid: (!names(&LID_ARGV).is_empty()).then(|| {
            text(&LID_CONF_ARGV)
                .and_then(|t| super::parse_lid_conf(&t))
                .unwrap_or(Lid::Sleep)
        }),
        sleep: text(&SLEEP_ARGV).is_some_and(|s| s.split_whitespace().any(|w| w == "mem")),
        wlans,
    }
}

/// power-profiles-daemon profile. On battery `balanced` already prefers efficiency
/// (power-profiles-daemon picks `balance_power` for the CPU).
pub fn profile_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Auto => "balanced",
        Mode::Low => "power-saver",
        Mode::High => "performance",
    }
}

/// The energy preference the profile results in, 0 = performance .. 100 = efficiency (Intel's
/// presets: balance_performance 50, balance_power 75, power 100). Shown in status.
pub fn epp(mode: Mode, source: Source) -> u8 {
    match (mode, source) {
        (Mode::Auto, Source::Ac) => 50,
        (Mode::Auto, Source::Battery) => 75,
        (Mode::Low, _) => 100,
        (Mode::High, _) => 0,
    }
}

/// 802.11 power save: on battery, unless High Power Mode.
pub fn wifi_power_save(mode: Mode, source: Source) -> bool {
    source == Source::Battery && mode != Mode::High
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// New power-profiles-daemon profile (only when it changes).
    pub profile: Option<&'static str>,
    /// (interface, power save on).
    pub wifi: Vec<(String, bool)>,
}

pub fn plan(f: &Facts, mode: Mode) -> Plan {
    let want = profile_name(mode);
    let ps = wifi_power_save(mode, f.source);
    Plan {
        profile: (f.cpu == Cpu::Epp && f.profile.as_deref() != Some(want)).then_some(want),
        wifi: f
            .wlans
            .iter()
            .filter(|w| w.power_save)
            .map(|w| (w.name.clone(), ps))
            .collect(),
    }
}

impl Plan {
    /// Fixed argv in order. Empty: nothing to change.
    pub fn argv(&self) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        if let Some(p) = self.profile {
            out.push(vec!["powerprofilesctl".into(), "set".into(), p.into()]);
        }
        for (i, on) in &self.wifi {
            out.push(
                [
                    "iw",
                    "dev",
                    i,
                    "set",
                    "power_save",
                    if *on { "on" } else { "off" },
                ]
                .map(String::from)
                .to_vec(),
            );
        }
        out
    }
}

/// `wol power status --json`, Settings > Battery, Control Center.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub source: Source,
    pub mode: Mode,
    /// What closing the lid does now; None without a lid.
    pub lid: Option<Lid>,
    pub battery: Option<u8>,
    pub cpu: Cpu,
    /// Energy preference the profile gives now ([`epp`]); None when nothing steers the CPU.
    pub epp: Option<u8>,
    /// Always None on Linux: cpuidle picks idle states. Kept for older clients.
    pub cx_lowest: Option<String>,
    /// The machine can sleep (suspend to RAM).
    pub sleep: bool,
    /// Interfaces with 802.11 power save on now.
    pub wifi_power_save: Vec<String>,
    /// Battery charge limit, %: not read yet.
    pub charge_limit: Option<u8>,
}

pub fn status(f: &Facts, st: &State) -> Status {
    let p = plan(f, st.mode);
    Status {
        source: f.source,
        mode: st.mode,
        lid: f.lid,
        battery: f.battery,
        cpu: f.cpu,
        epp: (f.cpu == Cpu::Epp).then(|| epp(st.mode, f.source)),
        cx_lowest: None,
        sleep: f.sleep,
        wifi_power_save: p.wifi.iter().filter(|w| w.1).map(|w| w.0.clone()).collect(),
        charge_limit: None,
    }
}

/// Low battery warnings at these levels (%), once each on the way down.
pub const WARN_AT: [u8; 3] = [20, 10, 5];
/// At or below this, discharging: lock and sleep before the battery dies.
pub const SLEEP_AT: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alert {
    /// Warn at this level (20, 10 or 5).
    Low(u8),
    /// Lock and sleep now.
    Sleep,
}

/// The lowest level crossed: 3 (sleep), 5, 10, 20, or None above 20.
fn level(percent: u8) -> Option<u8> {
    if percent <= SLEEP_AT {
        return Some(SLEEP_AT);
    }
    WARN_AT.iter().rev().copied().find(|w| percent <= *w)
}

/// One battery reading -> alert to show and the level to remember. `last` is the level already
/// alerted (None after charging). Each level fires once on the way down; charging resets.
pub fn battery_alert(
    last: Option<u8>,
    percent: u8,
    discharging: bool,
) -> (Option<Alert>, Option<u8>) {
    if !discharging {
        return (None, None);
    }
    let Some(l) = level(percent) else {
        return (None, last);
    };
    if last.is_some_and(|x| x <= l) {
        return (None, last);
    }
    let a = if l == SLEEP_AT {
        Alert::Sleep
    } else {
        Alert::Low(l)
    };
    (Some(a), Some(l))
}

/// hypridle listeners (seconds idle) that only act on battery: display off, then sleep.
pub const BATTERY_DISPLAY_OFF_SECS: u32 = 120;
pub const BATTERY_SLEEP_SECS: u32 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Idle {
    DisplayOff,
    Sleep,
}

impl Idle {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "display-off" => Some(Idle::DisplayOff),
            "sleep" => Some(Idle::Sleep),
            _ => None,
        }
    }
}

/// Should this battery-only idle step act now? On AC nothing happens (the lock listener turns the
/// display off; WolOS never sleeps on its own on AC). Sleep needs S3.
pub fn idle_acts(i: Idle, source: Source, sleep: bool) -> bool {
    source == Source::Battery && (i == Idle::DisplayOff || sleep)
}

/// hypridle listener blocks for [`idle_acts`].
pub fn hypridle_listeners() -> String {
    format!(
        "listener {{\n    timeout = {BATTERY_DISPLAY_OFF_SECS}\n    on-timeout = wol power idle display-off\n    on-resume = hyprctl dispatch dpms on\n}}\nlistener {{\n    timeout = {BATTERY_SLEEP_SECS}\n    on-timeout = wol power idle sleep\n}}\n"
    )
}

/// Login accounts start here; the login window (`_wollogin`, 911) is never locked.
pub const FIRST_USER_UID: u32 = 1000;
/// Session runtime dirs (`/run/user/<uid>`, made by systemd-logind).
pub const RUNTIME_ROOT: &str = "/run/user";
/// Hyprland IPC request: start the user's lock unless it already runs (as the user, in the session).
pub const LOCK_REQUEST: &[u8] = b"dispatch exec pgrep -x wol-lock || wol-lock";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub uid: u32,
    /// `<runtime>/hypr/<instance>/.socket.sock`.
    pub socket: String,
}

/// Desktop sessions of login accounts: every Hyprland instance under each runtime dir.
/// `list`: full paths of a directory's entries.
pub fn sessions(list: &dyn Fn(&str) -> Vec<String>) -> Vec<Session> {
    let mut out = Vec::new();
    for dir in list(RUNTIME_ROOT) {
        let Some(uid) = dir
            .rsplit('/')
            .next()
            .and_then(|n| n.parse::<u32>().ok())
            .filter(|u| *u >= FIRST_USER_UID)
        else {
            continue;
        };
        for inst in list(&format!("{dir}/hypr")) {
            let sock = format!("{inst}/.socket.sock");
            if list(&inst).contains(&sock) {
                out.push(Session { uid, socket: sock });
            }
        }
    }
    out
}

/// Home directory of `uid` in passwd(5) text.
pub fn home_of(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 7 && f[2].parse() == Ok(uid) && f[5].starts_with('/')).then(|| f[5].to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (probe argv, stdout) pairs; anything else fails like a missing file.
    fn probe(t: &'static [(&'static str, &'static str)]) -> impl Fn(&[&str]) -> Option<String> {
        move |argv| {
            let l = argv.join(" ");
            t.iter().find(|x| x.0 == l).map(|x| x.1.to_string())
        }
    }

    const LAPTOP: &[(&str, &str)] = &[
        ("nproc", "4\n"),
        ("powerprofilesctl get", "balanced\n"),
        (
            "ls /sys/class/power_supply",
            "ADP0\nBAT0\nhidpp_battery_0\n",
        ),
        ("cat /sys/class/power_supply/ADP0/type", "Mains\n"),
        ("cat /sys/class/power_supply/ADP0/online", "0\n"),
        (
            "cat /sys/class/power_supply/hidpp_battery_0/type",
            "Battery\n",
        ),
        (
            "cat /sys/class/power_supply/hidpp_battery_0/scope",
            "Device\n",
        ),
        ("cat /sys/class/power_supply/BAT0/type", "Battery\n"),
        ("cat /sys/class/power_supply/BAT0/capacity", "64\n"),
        ("ls /proc/acpi/button/lid", "LID0\n"),
        ("cat /sys/power/state", "freeze mem disk\n"),
        ("ls /sys/class/net", "lo\nenp0s31f6\nwlan0\n"),
        ("ls /sys/class/net/wlan0/wireless", ""),
        ("iw dev wlan0 get power_save", "Power save: off\n"),
    ];

    const DESKTOP: &[(&str, &str)] = &[
        ("nproc", "16\n"),
        ("powerprofilesctl get", "performance\n"),
        ("cat /sys/power/state", "freeze mem disk\n"),
    ];

    #[test]
    fn facts_from_probes() {
        let f = facts(&probe(LAPTOP));
        assert_eq!(
            f,
            Facts {
                ncpu: 4,
                cpu: Cpu::Epp,
                profile: Some("balanced".into()),
                source: Source::Battery,
                battery: Some(64),
                lid: Some(Lid::Sleep),
                sleep: true,
                wlans: vec![Wlan {
                    name: "wlan0".into(),
                    power_save: true
                }],
            }
        );
        let d = facts(&probe(DESKTOP));
        assert_eq!(
            (d.cpu, d.source, d.battery, d.lid, d.sleep),
            (Cpu::Epp, Source::Ac, None, None, true)
        );
        // A VM: nothing answers.
        let vm = facts(&|_: &[&str]| None);
        assert_eq!(
            (vm.cpu, vm.source, vm.sleep),
            (Cpu::Fixed, Source::Ac, false)
        );
        // Only plain names reach argv; no power save answer, no power save.
        let odd = facts(&probe(&[
            ("ls /sys/class/net", "wlp2s0 -rf ../x\n"),
            ("ls /sys/class/net/wlp2s0/wireless", ""),
            ("ls /sys/class/power_supply", "BAT1\n"),
            ("cat /sys/class/power_supply/BAT1/type", "Battery\n"),
            ("cat /sys/class/power_supply/BAT1/capacity", "250\n"),
            ("ls /proc/acpi/button/lid", "LID\n"),
            (
                "cat /etc/systemd/logind.conf.d/50-wolos-lid.conf",
                "[Login]\nHandleLidSwitch=ignore\n",
            ),
        ]));
        assert_eq!(
            odd.wlans,
            vec![Wlan {
                name: "wlp2s0".into(),
                power_save: false
            }]
        );
        assert_eq!(odd.battery, None, "out of range");
        assert_eq!(odd.source, Source::Ac, "no Mains supply: AC");
        assert_eq!(odd.lid, Some(Lid::Nothing));
    }

    #[test]
    fn profile_table() {
        use Mode::*;
        use Source::*;
        for (m, s, e, ps) in [
            (Auto, Ac, 50, false),
            (Auto, Battery, 75, true),
            (Low, Ac, 100, false),
            (Low, Battery, 100, true),
            (High, Ac, 0, false),
            (High, Battery, 0, false),
        ] {
            assert_eq!((epp(m, s), wifi_power_save(m, s)), (e, ps), "{m:?} {s:?}");
        }
        assert_eq!(
            Mode::ALL.map(profile_name),
            ["balanced", "power-saver", "performance"]
        );
    }

    fn shown(p: &Plan) -> String {
        p.argv()
            .iter()
            .map(|a| a.join(" "))
            .collect::<Vec<_>>()
            .join("; ")
    }

    #[test]
    fn plans_per_machine() {
        let laptop = facts(&probe(LAPTOP));
        assert_eq!(
            shown(&plan(&laptop, Mode::Auto)),
            "iw dev wlan0 set power_save on",
            "already balanced"
        );
        let mut on_ac = laptop.clone();
        on_ac.source = Source::Ac;
        assert_eq!(
            shown(&plan(&on_ac, Mode::Low)),
            "powerprofilesctl set power-saver; iw dev wlan0 set power_save off"
        );
        assert_eq!(
            shown(&plan(&facts(&probe(DESKTOP)), Mode::Auto)),
            "powerprofilesctl set balanced"
        );
        assert!(
            plan(&facts(&|_: &[&str]| None), Mode::Low)
                .argv()
                .is_empty()
        );
    }

    #[test]
    fn state_round_trip_and_defaults() {
        assert_eq!(State::parse(None), State::default());
        assert_eq!(State::parse(Some("junk")), State::default());
        let s = State { mode: Mode::Low };
        assert_eq!(s.to_json(), "{\"mode\":\"low\"}\n");
        assert_eq!(State::parse(Some(&s.to_json())), s);
        assert_eq!(
            State::parse(Some(r#"{"mode":"high","x":1}"#)).mode,
            Mode::High
        );
        for m in Mode::ALL {
            assert_eq!(Mode::parse(m.name()), Some(m));
        }
        assert_eq!(Mode::parse("max"), None);
    }

    #[test]
    fn status_reports_what_the_plan_sets() {
        let st = status(&facts(&probe(LAPTOP)), &State::default());
        assert_eq!(
            st,
            Status {
                source: Source::Battery,
                mode: Mode::Auto,
                lid: Some(Lid::Sleep),
                battery: Some(64),
                cpu: Cpu::Epp,
                epp: Some(75),
                cx_lowest: None,
                sleep: true,
                wifi_power_save: vec!["wlan0".into()],
                charge_limit: None,
            }
        );
        let json = serde_json::to_value(&st).unwrap();
        assert_eq!(json["source"], "battery");
        assert_eq!(json["cpu"], "epp");
        let vm = status(&facts(&|_: &[&str]| None), &State::default());
        assert_eq!((vm.lid, vm.epp, vm.cpu), (None, None, Cpu::Fixed));
    }

    #[test]
    fn sessions_to_lock_before_sleep() {
        let tree = [
            "/run/user/911/hypr/g/.socket.sock",
            "/run/user/1001/hypr/a1/.socket.sock",
            "/run/user/1001/hypr/a1/.socket2.sock",
            "/run/user/1001/hypr/stale/hyprland.log",
            "/run/user/1002/hypr/b/.socket.sock",
            "/run/user/x/hypr/c/.socket.sock",
        ];
        let list = |d: &str| -> Vec<String> {
            let mut v: Vec<String> = tree
                .iter()
                .filter_map(|p| p.strip_prefix(&format!("{d}/")))
                .map(|r| format!("{d}/{}", r.split('/').next().unwrap()))
                .collect();
            v.sort();
            v.dedup();
            v
        };
        assert_eq!(
            sessions(&list),
            [
                Session {
                    uid: 1001,
                    socket: "/run/user/1001/hypr/a1/.socket.sock".into()
                },
                Session {
                    uid: 1002,
                    socket: "/run/user/1002/hypr/b/.socket.sock".into()
                },
            ],
            "not the login window, not a stale instance"
        );
        let pw = "root:*:0:0::/root:/bin/sh\nann:*:1001:1001:Ann:/home/ann:/usr/bin/zsh\n";
        assert_eq!(home_of(pw, 1001).as_deref(), Some("/home/ann"));
        assert_eq!(home_of(pw, 1002), None);
        assert_eq!(LOCK_REQUEST, b"dispatch exec pgrep -x wol-lock || wol-lock");
    }

    #[test]
    fn battery_alerts_fire_once_per_level() {
        // (last alerted, percent, discharging) -> (alert, remembered)
        for (last, pct, dis, want, keep) in [
            (None, 50, true, None, None),
            (None, 20, true, Some(Alert::Low(20)), Some(20)),
            (Some(20), 19, true, None, Some(20)),
            (Some(20), 10, true, Some(Alert::Low(10)), Some(10)),
            (Some(20), 7, true, Some(Alert::Low(10)), Some(10)),
            (Some(10), 5, true, Some(Alert::Low(5)), Some(5)),
            (Some(5), 3, true, Some(Alert::Sleep), Some(3)),
            (Some(3), 2, true, None, Some(3)),
            (None, 2, true, Some(Alert::Sleep), Some(3)),
            (Some(5), 4, false, None, None),
            (Some(10), 30, true, None, Some(10)),
        ] {
            assert_eq!(
                battery_alert(last, pct, dis),
                (want, keep),
                "{last:?} {pct} {dis}"
            );
        }
    }

    #[test]
    fn idle_steps_act_on_battery_only() {
        for (i, s, sleep, want) in [
            (Idle::DisplayOff, Source::Battery, false, true),
            (Idle::DisplayOff, Source::Ac, true, false),
            (Idle::Sleep, Source::Battery, true, true),
            (Idle::Sleep, Source::Battery, false, false),
            (Idle::Sleep, Source::Ac, true, false),
        ] {
            assert_eq!(idle_acts(i, s, sleep), want, "{i:?} {s:?} {sleep}");
        }
        assert_eq!(Idle::parse("sleep"), Some(Idle::Sleep));
        assert_eq!(Idle::parse("reboot"), None);
        let l = hypridle_listeners();
        assert!(l.contains("timeout = 120\n    on-timeout = wol power idle display-off"));
        assert!(l.contains("timeout = 600\n    on-timeout = wol power idle sleep"));
        assert!(
            !l.contains("wol-lock"),
            "the lock listener stays the Lock pane's"
        );
    }
}
