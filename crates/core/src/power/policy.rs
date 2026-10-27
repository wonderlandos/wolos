//! Power policy: the power source picks the profile, Low Power / High Power Mode is the user's
//! override. Pure: facts from read-only probes + the saved state in, a plan of fixed argv out.
//! `wol power apply` (devd on AC changes, rc.d at boot and resume) and wol-admind `power-mode` run
//! the same plan. The lid action is wol-admind `lid` (kernel `hw.acpi.lid_switch_state`).
use super::{EPP_ARGV, NCPU_ARGV, POWERD_ENABLED_ARGV, epp_key};
use serde::{Deserialize, Serialize};

/// World-readable; wol-admind `power-mode` writes it, `wol power apply` reads it.
pub const STATE_FILE: &str = "/usr/local/etc/wolos/power.json";

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
    /// Mains, or a desktop without an AC adapter sysctl.
    #[default]
    Ac,
    Battery,
}

/// Who steers the CPU clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cpu {
    /// Intel Speed Shift: the energy/performance preference (`dev.hwpstate_intel.N.epp`).
    Epp,
    /// cpufreq(4) without HWP (AMD hwpstate, Intel est): powerd(8) picks the speed.
    Powerd,
    /// Nothing to steer (virtual machine, firmware owns it).
    #[default]
    Fixed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wlan {
    pub name: String,
    /// The driver supports 802.11 power save (`ifconfig wlanN list caps` has PMGT).
    pub power_save: bool,
}

/// What the probes found. Absent sysctls mean the hardware is absent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub ncpu: u32,
    pub cpu: Cpu,
    /// Current `powerd_flags` (rc.conf), when powerd runs.
    pub powerd_flags: Option<String>,
    pub source: Source,
    /// `hw.acpi.battery.life`, None without a battery.
    pub battery: Option<u8>,
    /// Current `hw.acpi.cpu.cx_lowest`; None without acpi_cpu(4) idle states.
    pub cx_lowest: Option<String>,
    /// Current `hw.acpi.lid_switch_state`; None without a lid.
    pub lid_switch: Option<String>,
    /// S3 in `hw.acpi.supported_sleep_state`.
    pub sleep: bool,
    pub wlans: Vec<Wlan>,
}

pub const ACLINE_ARGV: [&str; 3] = ["sysctl", "-n", "hw.acpi.acline"];
pub const BATTERY_ARGV: [&str; 3] = ["sysctl", "-n", "hw.acpi.battery.life"];
pub const CX_ARGV: [&str; 3] = ["sysctl", "-n", "hw.acpi.cpu.cx_lowest"];
pub const LID_ARGV: [&str; 3] = ["sysctl", "-n", "hw.acpi.lid_switch_state"];
pub const SLEEP_ARGV: [&str; 3] = ["sysctl", "-n", "hw.acpi.supported_sleep_state"];
pub const POWERD_FLAGS_ARGV: [&str; 3] = ["sysrc", "-n", "powerd_flags"];
pub const WLAN_ARGV: [&str; 3] = ["ifconfig", "-g", "wlan"];

/// `wlanN` only (the name reaches argv).
fn is_wlan(s: &str) -> bool {
    s.len() > 4
        && s.len() < 9
        && s.starts_with("wlan")
        && s[4..].bytes().all(|b| b.is_ascii_digit())
}

/// `probe`: fixed argv -> stdout, None when it failed (a missing sysctl fails).
pub fn facts(probe: &dyn Fn(&[&str]) -> Option<String>) -> Facts {
    let text = |argv: &[&str]| probe(argv).map(|s| s.trim().to_owned());
    let num = |argv: &[&str]| text(argv).and_then(|s| s.parse::<i64>().ok());
    let cpu = if num(&EPP_ARGV).is_some() {
        Cpu::Epp
    } else if probe(&POWERD_ENABLED_ARGV).is_some() {
        Cpu::Powerd
    } else {
        Cpu::Fixed
    };
    let wlans = text(&WLAN_ARGV)
        .unwrap_or_default()
        .split_whitespace()
        .filter(|n| is_wlan(n))
        .map(|n| Wlan {
            name: n.to_owned(),
            power_save: probe(&["ifconfig", n, "list", "caps"]).is_some_and(|c| {
                c.split(|ch: char| !ch.is_ascii_alphanumeric())
                    .any(|w| w == "PMGT")
            }),
        })
        .collect();
    Facts {
        ncpu: num(&NCPU_ARGV).map_or(0, |n| n.clamp(0, 1024) as u32),
        cpu,
        powerd_flags: (cpu == Cpu::Powerd)
            .then(|| text(&POWERD_FLAGS_ARGV))
            .flatten(),
        source: match num(&ACLINE_ARGV) {
            Some(0) => Source::Battery,
            _ => Source::Ac,
        },
        battery: num(&BATTERY_ARGV)
            .filter(|l| (0..=100).contains(l))
            .map(|l| l as u8),
        cx_lowest: text(&CX_ARGV).filter(|s| !s.is_empty()),
        lid_switch: text(&LID_ARGV).filter(|s| !s.is_empty()),
        sleep: text(&SLEEP_ARGV).is_some_and(|s| s.split_whitespace().any(|w| w == "S3")),
        wlans,
    }
}

/// Speed Shift energy preference, 0 = performance .. 100 = efficiency. Intel's own presets:
/// balance_performance 128/255 (50), balance_power 192/255 (75), power 255/255 (100).
pub fn epp(mode: Mode, source: Source) -> u8 {
    match (mode, source) {
        (Mode::Auto, Source::Ac) => 50,
        (Mode::Auto, Source::Battery) => 75,
        (Mode::Low, Source::Ac) => 85,
        (Mode::Low, Source::Battery) => 100,
        (Mode::High, _) => 0,
    }
}

/// powerd(8) flags. powerd itself switches between `-a` (AC) and `-b` (battery).
pub fn powerd_flags(mode: Mode) -> &'static str {
    use crate::controls::{POWERD_AUTO, POWERD_LOW};
    match mode {
        Mode::Auto => POWERD_AUTO,
        Mode::Low => POWERD_LOW,
        Mode::High => POWERD_HIGH,
    }
}

pub const POWERD_HIGH: &str = "-a maximum -b hiadaptive";

/// Deepest CPU idle state. FreeBSD keeps C1 on AC (`performance_cx_lowest=NONE`) and Cmax on
/// battery (`economy_cx_lowest`); WolOS also idles deep on AC (quiet desktops) except in High
/// Power Mode on AC, where C1 keeps wake-up latency lowest. Matches power_profile(8) on battery.
pub fn cx_lowest(mode: Mode, source: Source) -> &'static str {
    match (mode, source) {
        (Mode::High, Source::Ac) => "C1",
        _ => "Cmax",
    }
}

/// 802.11 power save: on battery, unless High Power Mode.
pub fn wifi_power_save(mode: Mode, source: Source) -> bool {
    source == Source::Battery && mode != Mode::High
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// `key=value` for one `sysctl` call (every key exists on this machine).
    pub sysctl: Vec<(String, String)>,
    /// New `powerd_flags`, then `service powerd restart` (only when powerd runs and they change).
    pub powerd_flags: Option<&'static str>,
    /// (interface, power save on).
    pub wifi: Vec<(String, bool)>,
}

pub fn plan(f: &Facts, mode: Mode) -> Plan {
    let mut sysctl = Vec::new();
    if f.cpu == Cpu::Epp {
        let v = epp(mode, f.source).to_string();
        sysctl.extend((0..f.ncpu.max(1)).map(|c| (epp_key(c), v.clone())));
    }
    let cx = cx_lowest(mode, f.source);
    if f.cx_lowest.as_deref().is_some_and(|c| c != cx) {
        sysctl.push(("hw.acpi.cpu.cx_lowest".into(), cx.into()));
    }
    let flags = powerd_flags(mode);
    let ps = wifi_power_save(mode, f.source);
    Plan {
        sysctl,
        powerd_flags: (f.cpu == Cpu::Powerd && f.powerd_flags.as_deref() != Some(flags))
            .then_some(flags),
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
        if !self.sysctl.is_empty() {
            let mut v = vec!["sysctl".to_owned()];
            v.extend(self.sysctl.iter().map(|(k, x)| format!("{k}={x}")));
            out.push(v);
        }
        if let Some(f) = self.powerd_flags {
            out.push(vec!["sysrc".into(), format!("powerd_flags={f}")]);
            out.push(vec!["service".into(), "powerd".into(), "restart".into()]);
        }
        for (i, on) in &self.wifi {
            out.push(vec![
                "ifconfig".into(),
                i.clone(),
                if *on { "powersave" } else { "-powersave" }.into(),
            ]);
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
    pub lid: Option<super::Lid>,
    pub battery: Option<u8>,
    pub cpu: Cpu,
    /// Preference the plan sets now (Speed Shift only).
    pub epp: Option<u8>,
    pub cx_lowest: Option<String>,
    /// The machine can sleep (S3).
    pub sleep: bool,
    /// Interfaces with 802.11 power save on now.
    pub wifi_power_save: Vec<String>,
    /// Battery charge limit, %: None, no FreeBSD 15.1 driver exposes one.
    pub charge_limit: Option<u8>,
}

pub fn status(f: &Facts, st: &State) -> Status {
    let p = plan(f, st.mode);
    Status {
        source: f.source,
        mode: st.mode,
        lid: f.lid_switch.as_deref().map(|l| {
            if l == "NONE" {
                super::Lid::Nothing
            } else {
                super::Lid::Sleep
            }
        }),
        battery: f.battery,
        cpu: f.cpu,
        epp: (f.cpu == Cpu::Epp).then(|| epp(st.mode, f.source)),
        cx_lowest: f
            .cx_lowest
            .as_ref()
            .map(|_| cx_lowest(st.mode, f.source).to_owned()),
        sleep: f.sleep,
        wifi_power_save: p.wifi.iter().filter(|w| w.1).map(|w| w.0.clone()).collect(),
        charge_limit: None,
    }
}

/// Laptops sleep when the lid closes, like a Mac: the first `wol power apply` sets S3 when the
/// machine has a lid, can sleep, and nobody chose (no lid line in `sysctl.conf.local`, which
/// wol-admind `lid` writes). The lock is on screen before the machine sleeps (`wol power suspend`).
pub fn lid_default(f: &Facts, sysctl_local: Option<&str>) -> Option<(String, String)> {
    let key = "hw.acpi.lid_switch_state";
    let chosen = sysctl_local.unwrap_or("").lines().any(|l| {
        !l.trim_start().starts_with('#') && l.split_once('=').is_some_and(|(k, _)| k.trim() == key)
    });
    (f.sleep && f.lid_switch.as_deref() == Some("NONE") && !chosen).then(|| {
        (
            key.to_owned(),
            super::lid_value(super::Lid::Sleep).to_owned(),
        )
    })
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
/// Session runtime dirs (`/var/run/user/<uid>`, made by wol-login).
pub const RUNTIME_ROOT: &str = "/var/run/user";
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

    /// (probe argv, stdout) pairs; anything else fails like a missing sysctl.
    fn probe(t: &'static [(&'static str, &'static str)]) -> impl Fn(&[&str]) -> Option<String> {
        move |argv| {
            let l = argv.join(" ");
            t.iter().find(|x| x.0 == l).map(|x| x.1.to_string())
        }
    }

    const LAPTOP: &[(&str, &str)] = &[
        ("sysctl -n hw.ncpu", "4\n"),
        ("sysctl -n dev.hwpstate_intel.0.epp", "50\n"),
        ("sysctl -n hw.acpi.acline", "0\n"),
        ("sysctl -n hw.acpi.battery.life", "64\n"),
        ("sysctl -n hw.acpi.cpu.cx_lowest", "C1\n"),
        ("sysctl -n hw.acpi.lid_switch_state", "NONE\n"),
        ("sysctl -n hw.acpi.supported_sleep_state", "S3 S4 S5\n"),
        ("ifconfig -g wlan", "wlan0\n"),
        (
            "ifconfig wlan0 list caps",
            "drivercaps=4f8def41<STA,FF,TURBOP,IBSS,PMGT,SWRETRY,TXPMGT,SHSLOT>\n",
        ),
    ];

    const AMD_DESKTOP: &[(&str, &str)] = &[
        ("sysctl -n hw.ncpu", "16\n"),
        ("service powerd enabled", ""),
        ("sysrc -n powerd_flags", "-a hiadaptive\n"),
        ("sysctl -n hw.acpi.cpu.cx_lowest", "C1\n"),
        ("sysctl -n hw.acpi.supported_sleep_state", "S3 S4 S5\n"),
    ];

    #[test]
    fn facts_from_probes() {
        let f = facts(&probe(LAPTOP));
        assert_eq!(
            f,
            Facts {
                ncpu: 4,
                cpu: Cpu::Epp,
                powerd_flags: None,
                source: Source::Battery,
                battery: Some(64),
                cx_lowest: Some("C1".into()),
                lid_switch: Some("NONE".into()),
                sleep: true,
                wlans: vec![Wlan {
                    name: "wlan0".into(),
                    power_save: true
                }],
            }
        );
        let d = facts(&probe(AMD_DESKTOP));
        assert_eq!(
            (
                d.cpu,
                d.source,
                d.battery,
                d.lid_switch,
                d.powerd_flags.as_deref()
            ),
            (Cpu::Powerd, Source::Ac, None, None, Some("-a hiadaptive"))
        );
        // A VM: nothing answers.
        let vm = facts(&|_: &[&str]| None);
        assert_eq!(
            (vm.cpu, vm.source, vm.sleep),
            (Cpu::Fixed, Source::Ac, false)
        );
        // Only wlanN names reach argv; no PMGT, no power save.
        let odd = facts(&probe(&[
            ("ifconfig -g wlan", "wlan1 em0;id wlan12345678\n"),
            ("ifconfig wlan1 list caps", "drivercaps=1<STA,TXPMGT>\n"),
            ("sysctl -n hw.acpi.battery.life", "-1\n"),
        ]));
        assert_eq!(
            odd.wlans,
            vec![Wlan {
                name: "wlan1".into(),
                power_save: false
            }]
        );
        assert_eq!(odd.battery, None, "-1 = no battery");
    }

    #[test]
    fn profile_table() {
        use Mode::*;
        use Source::*;
        for (m, s, e, cx, ps) in [
            (Auto, Ac, 50, "Cmax", false),
            (Auto, Battery, 75, "Cmax", true),
            (Low, Ac, 85, "Cmax", false),
            (Low, Battery, 100, "Cmax", true),
            (High, Ac, 0, "C1", false),
            (High, Battery, 0, "Cmax", false),
        ] {
            assert_eq!(
                (epp(m, s), cx_lowest(m, s), wifi_power_save(m, s)),
                (e, cx, ps),
                "{m:?} {s:?}"
            );
        }
        assert_eq!(powerd_flags(Auto), "-a hiadaptive");
        assert_eq!(powerd_flags(Low), "-a minimum -b minimum");
        assert_eq!(powerd_flags(High), "-a maximum -b hiadaptive");
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
            "sysctl dev.hwpstate_intel.0.epp=75 dev.hwpstate_intel.1.epp=75 \
             dev.hwpstate_intel.2.epp=75 dev.hwpstate_intel.3.epp=75 hw.acpi.cpu.cx_lowest=Cmax; \
             ifconfig wlan0 powersave"
        );
        let mut on_ac = laptop.clone();
        on_ac.source = Source::Ac;
        on_ac.cx_lowest = Some("Cmax".into());
        assert_eq!(
            shown(&plan(&on_ac, Mode::High)),
            "sysctl dev.hwpstate_intel.0.epp=0 dev.hwpstate_intel.1.epp=0 \
             dev.hwpstate_intel.2.epp=0 dev.hwpstate_intel.3.epp=0 hw.acpi.cpu.cx_lowest=C1; \
             ifconfig wlan0 -powersave"
        );
        // powerd: flags change only when needed; never `service powerd restart` without powerd.
        let amd = facts(&probe(AMD_DESKTOP));
        assert_eq!(
            shown(&plan(&amd, Mode::Auto)),
            "sysctl hw.acpi.cpu.cx_lowest=Cmax"
        );
        assert_eq!(
            shown(&plan(&amd, Mode::Low)),
            "sysctl hw.acpi.cpu.cx_lowest=Cmax; sysrc powerd_flags=-a minimum -b minimum; \
             service powerd restart"
        );
        for m in Mode::ALL {
            assert!(
                !shown(&plan(&laptop, m)).contains("powerd"),
                "Speed Shift: powerd is off"
            );
        }
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
                lid: Some(crate::power::Lid::Nothing),
                battery: Some(64),
                cpu: Cpu::Epp,
                epp: Some(75),
                cx_lowest: Some("Cmax".into()),
                sleep: true,
                wifi_power_save: vec!["wlan0".into()],
                charge_limit: None,
            }
        );
        let json = serde_json::to_value(&st).unwrap();
        assert_eq!(json["source"], "battery");
        assert_eq!(json["cpu"], "epp");
        let d = status(&facts(&probe(AMD_DESKTOP)), &State::default());
        assert_eq!((d.lid, d.epp, d.cpu), (None, None, Cpu::Powerd));
    }

    #[test]
    fn lid_sleeps_by_default_unless_chosen() {
        let f = facts(&probe(LAPTOP));
        let s3 = Some(("hw.acpi.lid_switch_state".to_string(), "S3".to_string()));
        assert_eq!(lid_default(&f, None), s3);
        assert_eq!(
            lid_default(&f, Some("kern.coredump=0\n# hw.acpi.lid_switch_state=S3\n")),
            s3
        );
        assert_eq!(
            lid_default(&f, Some("hw.acpi.lid_switch_state=NONE\n")),
            None,
            "the user chose Nothing"
        );
        let mut no_s3 = f.clone();
        no_s3.sleep = false;
        assert_eq!(lid_default(&no_s3, None), None);
        let mut desktop = f.clone();
        desktop.lid_switch = None;
        assert_eq!(lid_default(&desktop, None), None);
        let mut set = f;
        set.lid_switch = Some("S3".into());
        assert_eq!(lid_default(&set, None), None, "already sleeps");
    }

    #[test]
    fn sessions_to_lock_before_sleep() {
        let tree = [
            "/var/run/user/911/hypr/g/.socket.sock",
            "/var/run/user/1001/hypr/a1/.socket.sock",
            "/var/run/user/1001/hypr/a1/.socket2.sock",
            "/var/run/user/1001/hypr/stale/hyprland.log",
            "/var/run/user/1002/hypr/b/.socket.sock",
            "/var/run/user/x/hypr/c/.socket.sock",
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
                    socket: "/var/run/user/1001/hypr/a1/.socket.sock".into()
                },
                Session {
                    uid: 1002,
                    socket: "/var/run/user/1002/hypr/b/.socket.sock".into()
                },
            ],
            "not the login window, not a stale instance"
        );
        let pw = "root:*:0:0::/root:/bin/sh\nann:*:1001:1001:Ann:/home/ann:/usr/local/bin/zsh\n";
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
