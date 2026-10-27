//! Battery details, energy profile (hwpstate_intel EPP) and lid action for Settings > Battery.
//! The automatic AC/battery policy is [`policy`].
use crate::system::Host;

pub mod policy;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Charge {
    #[default]
    Discharging,
    Charging,
    Full,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BatteryInfo {
    pub percent: u8,
    pub charge: Charge,
    /// Last full capacity / design capacity, %.
    pub health: Option<u8>,
    pub cycles: Option<u32>,
    pub minutes_left: Option<u32>,
    pub design_mwh: Option<u32>,
    pub full_mwh: Option<u32>,
}

/// First number of a value ("48000 mWh" -> 48000).
fn num(v: &str) -> Option<u32> {
    v.split_whitespace().next()?.parse().ok()
}

/// Full / design capacity as a percentage (capped at 100).
fn health(full: Option<u32>, design: Option<u32>) -> Option<u8> {
    let (f, d) = (full?, design?);
    (d > 0).then(|| (u64::from(f) * 100 / u64::from(d)).min(100) as u8)
}

/// FreeBSD `acpiconf -i 0`. None without a battery.
pub fn parse_acpiconf(text: &str) -> Option<BatteryInfo> {
    let p = crate::system::pairs(text, ':');
    if p.get("Present") == Some(&"no") {
        return None;
    }
    let percent = p
        .get("Remaining capacity")?
        .trim_end_matches('%')
        .trim()
        .parse::<u8>()
        .ok()?
        .min(100);
    let mwh = |k: &str| p.get(k).filter(|v| v.ends_with("mWh")).and_then(|v| num(v));
    let (design, full) = (
        p.get("Design capacity").and_then(|v| num(v)),
        p.get("Last full capacity").and_then(|v| num(v)),
    );
    let charge = match p.get("State").and_then(|s| s.split_whitespace().next()) {
        Some("charging") => Charge::Charging,
        Some("high" | "full") => Charge::Full,
        _ => Charge::Discharging,
    };
    let minutes_left = p.get("Remaining time").and_then(|t| {
        let (h, m) = t.split_once(':')?;
        Some(h.trim().parse::<u32>().ok()? * 60 + m.trim().parse::<u32>().ok()?)
    });
    Some(BatteryInfo {
        percent,
        charge,
        health: health(full, design),
        cycles: p.get("Cycle Count").and_then(|v| num(v)).filter(|c| *c > 0),
        minutes_left,
        design_mwh: mwh("Design capacity"),
        full_mwh: mwh("Last full capacity"),
    })
}

/// Linux `/sys/class/power_supply/BAT*` files by name (energy in µWh, charge in µAh).
pub fn parse_sysfs(files: impl Fn(&str) -> Option<String>) -> Option<BatteryInfo> {
    let get = |k: &str| files(k).and_then(|v| v.trim().parse::<u64>().ok());
    let percent = get("capacity")?.min(100) as u8;
    let charge = match files("status").as_deref().map(str::trim) {
        Some("Charging") => Charge::Charging,
        Some("Full" | "Not charging") => Charge::Full,
        _ => Charge::Discharging,
    };
    let mwh = |k: &str| get(k).map(|v| (v / 1000) as u32);
    let (design, full) = match (mwh("energy_full_design"), mwh("energy_full")) {
        (Some(d), Some(f)) => (Some(d), Some(f)),
        _ => (None, None),
    };
    let by_charge = health(
        get("charge_full").map(|v| (v / 1000) as u32),
        get("charge_full_design").map(|v| (v / 1000) as u32),
    );
    Some(BatteryInfo {
        percent,
        charge,
        health: health(full, design).or(by_charge),
        cycles: get("cycle_count").filter(|c| *c > 0).map(|c| c as u32),
        minutes_left: None,
        design_mwh: design,
        full_mwh: full,
    })
}

/// First battery, None without one.
pub fn battery(h: &dyn Host) -> Option<BatteryInfo> {
    if h.is_freebsd() {
        return h
            .run(&["acpiconf", "-i", "0"])
            .and_then(|t| parse_acpiconf(&t));
    }
    let dir = h
        .list_dir("/sys/class/power_supply")
        .into_iter()
        .find(|d| d.rsplit('/').next().is_some_and(|n| n.starts_with("BAT")))?;
    parse_sysfs(|k| h.read_text(&format!("{dir}/{k}")))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Profile {
    LowPower,
    Automatic,
    Performance,
}

/// hwpstate_intel energy/performance preference: 0 = performance .. 100 = efficiency.
pub const EPP: [(Profile, u8); 3] = [
    (Profile::LowPower, 85),
    (Profile::Automatic, 50),
    (Profile::Performance, 0),
];

/// Nearest shipped profile.
pub fn profile_of_epp(epp: u8) -> Profile {
    EPP.iter()
        .min_by_key(|(_, v)| v.abs_diff(epp))
        .map_or(Profile::Automatic, |(p, _)| *p)
}

pub fn epp_key(cpu: u32) -> String {
    format!("dev.hwpstate_intel.{cpu}.epp")
}

/// Current profile; None without a tunable (no Speed Shift / platform profile).
pub fn profile(h: &dyn Host) -> Option<Profile> {
    if h.is_freebsd() {
        return h
            .sysctl_i64(&epp_key(0))
            .map(|v| profile_of_epp(v.clamp(0, 100) as u8));
    }
    match h.read_text("/sys/firmware/acpi/platform_profile")?.trim() {
        "low-power" | "quiet" | "cool" => Some(Profile::LowPower),
        "balanced" => Some(Profile::Automatic),
        "performance" | "balanced-performance" => Some(Profile::Performance),
        _ => None,
    }
}

pub fn ncpu(h: &dyn Host) -> u32 {
    h.sysctl_i64("hw.ncpu")
        .filter(|n| *n > 0)
        .map_or(1, |n| n.min(i64::from(u32::MAX)) as u32)
}

/// What wol-admind finds before it plans Low Power Mode (probes: [`NCPU_ARGV`], [`EPP_ARGV`],
/// [`POWERD_ENABLED_ARGV`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerFacts {
    /// `hw.ncpu`; 0 when unknown.
    pub ncpu: u32,
    /// Speed Shift energy preference present (`dev.hwpstate_intel.0.epp`).
    pub epp: bool,
    /// powerd enabled (rc.conf or rc.conf.d/powerd, which the hardware module writes).
    pub powerd: bool,
}

pub const NCPU_ARGV: [&str; 3] = ["sysctl", "-n", "hw.ncpu"];
pub const EPP_ARGV: [&str; 3] = ["sysctl", "-n", "dev.hwpstate_intel.0.epp"];
/// Exits 0 when enabled.
pub const POWERD_ENABLED_ARGV: [&str; 3] = ["service", "powerd", "enabled"];

/// `probe`: fixed argv -> stdout, None when it failed.
pub fn power_facts(probe: &dyn Fn(&[&str]) -> Option<String>) -> PowerFacts {
    let num = |argv: &[&str]| probe(argv).and_then(|s| s.trim().parse::<u32>().ok());
    PowerFacts {
        ncpu: num(&NCPU_ARGV).unwrap_or(0),
        epp: num(&EPP_ARGV).is_some(),
        powerd: probe(&POWERD_ENABLED_ARGV).is_some(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Lid {
    Sleep,
    Nothing,
}

/// FreeBSD lid action; None without a lid (desktops) or for states Settings does not offer.
pub fn lid(h: &dyn Host) -> Option<Lid> {
    if !h.is_freebsd() {
        return None;
    }
    match h.sysctl_string("hw.acpi.lid_switch_state")?.trim() {
        "NONE" => Some(Lid::Nothing),
        "S1" | "S2" | "S3" | "S4" => Some(Lid::Sleep),
        _ => None,
    }
}

pub fn lid_value(l: Lid) -> &'static str {
    match l {
        Lid::Sleep => "S3",
        Lid::Nothing => "NONE",
    }
}

pub const SYSCTL_LOCAL: &str = "/etc/sysctl.conf.local";

/// `key=value` replaces the first line setting `key` (later duplicates dropped) or is appended;
/// every other line is kept as is.
pub fn upsert_sysctl(conf: Option<&str>, key: &str, value: &str) -> String {
    let mut out = String::new();
    let mut done = false;
    for l in conf.unwrap_or("").lines() {
        let sets = !l.trim_start().starts_with('#')
            && l.split_once('=').is_some_and(|(k, _)| k.trim() == key);
        if !sets {
            out.push_str(l);
            out.push('\n');
        } else if !done {
            out.push_str(&format!("{key}={value}\n"));
            done = true;
        }
    }
    if !done {
        out.push_str(&format!("{key}={value}\n"));
    }
    out
}

fn plain_number(s: &str, max: u32) -> bool {
    !s.is_empty()
        && s.len() <= 4
        && s.bytes().all(|b| b.is_ascii_digit())
        && (s == "0" || !s.starts_with('0'))
        && s.parse::<u32>().is_ok_and(|n| n <= max)
}

/// wol-admind allow-list: the lid action and per-CPU EPP, nothing else.
pub fn is_allowed_sysctl(key: &str, value: &str) -> bool {
    if key == "hw.acpi.lid_switch_state" {
        return value == "NONE" || value == "S3";
    }
    key.strip_prefix("dev.hwpstate_intel.")
        .and_then(|r| r.strip_suffix(".epp"))
        .is_some_and(|cpu| plain_number(cpu, 1023) && plain_number(value, 100))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    const ACPICONF: &str = "Design capacity:\t48000 mWh\nLast full capacity:\t41000 mWh\nTechnology:\tsecondary (rechargeable)\nDesign voltage:\t11400 mV\nCapacity (warn):\t2400 mWh\nCapacity (low):\t200 mWh\nCycle Count:\t212\nMfg Date:\t\nLow/warn granularity:\t10 mWh\nWarn/full granularity:\t10 mWh\nModel number:\t5B10W13930\nSerial number:\t 1234\nType:\tLiP\nOEM info:\tSMP\nState:\tdischarging\nRemaining capacity:\t82%\nRemaining time:\t3:25\nPresent rate:\t9850 mW\nPresent voltage:\t12100 mV\n";

    #[test]
    fn acpiconf_battery() {
        let b = parse_acpiconf(ACPICONF).unwrap();
        assert_eq!(
            b,
            BatteryInfo {
                percent: 82,
                charge: Charge::Discharging,
                health: Some(85),
                cycles: Some(212),
                minutes_left: Some(205),
                design_mwh: Some(48000),
                full_mwh: Some(41000),
            }
        );
        let charging = ACPICONF
            .replace("State:\tdischarging", "State:\tcharging")
            .replace("Remaining time:\t3:25", "Remaining time:\tunknown")
            .replace("Cycle Count:\t212", "Cycle Count:\t0");
        let b = parse_acpiconf(&charging).unwrap();
        assert_eq!(
            (b.charge, b.minutes_left, b.cycles),
            (Charge::Charging, None, None)
        );
        let full = ACPICONF
            .replace("State:\tdischarging", "State:\thigh")
            .replace("82%", "100%");
        assert_eq!(parse_acpiconf(&full).unwrap().charge, Charge::Full);
        let critical = ACPICONF.replace("State:\tdischarging", "State:\tdischarging critical");
        assert_eq!(
            parse_acpiconf(&critical).unwrap().charge,
            Charge::Discharging
        );
        assert_eq!(parse_acpiconf("Present:\tno\n"), None);
        assert_eq!(parse_acpiconf(""), None);
        assert_eq!(parse_acpiconf("garbage"), None);
    }

    #[test]
    fn sysfs_battery() {
        let files = |kv: &'static [(&'static str, &'static str)]| {
            move |k: &str| kv.iter().find(|x| x.0 == k).map(|x| x.1.to_string())
        };
        let b = parse_sysfs(files(&[
            ("capacity", "55\n"),
            ("status", "Charging\n"),
            ("energy_full", "41000000\n"),
            ("energy_full_design", "50000000\n"),
            ("cycle_count", "33\n"),
        ]))
        .unwrap();
        assert_eq!(
            (b.percent, b.charge, b.health, b.cycles),
            (55, Charge::Charging, Some(82), Some(33))
        );
        assert_eq!((b.design_mwh, b.full_mwh), (Some(50000), Some(41000)));
        let c = parse_sysfs(files(&[
            ("capacity", "100"),
            ("status", "Full"),
            ("charge_full", "3000000"),
            ("charge_full_design", "4000000"),
        ]))
        .unwrap();
        assert_eq!((c.charge, c.health), (Charge::Full, Some(75)));
        assert_eq!(parse_sysfs(files(&[("status", "Full")])), None);
    }

    #[test]
    fn battery_reads_per_platform() {
        let h = FakeHost::new(true);
        h.command("acpiconf -i 0", ACPICONF);
        assert_eq!(battery(&h).map(|b| b.percent), Some(82));
        assert_eq!(battery(&FakeHost::new(true)), None);
        let l = FakeHost::new(false);
        l.file("/sys/class/power_supply/AC/online", "1\n")
            .file("/sys/class/power_supply/BAT0/capacity", "40\n")
            .file("/sys/class/power_supply/BAT0/status", "Discharging\n");
        assert_eq!(
            battery(&l).map(|b| (b.percent, b.charge)),
            Some((40, Charge::Discharging))
        );
        assert_eq!(battery(&FakeHost::new(false)), None);
    }

    #[test]
    fn energy_profile() {
        for (epp, want) in [
            (0, Profile::Performance),
            (20, Profile::Performance),
            (50, Profile::Automatic),
            (70, Profile::LowPower),
            (100, Profile::LowPower),
        ] {
            assert_eq!(profile_of_epp(epp), want, "{epp}");
        }
        for (p, epp) in EPP {
            assert_eq!(profile_of_epp(epp), p);
        }
        assert_eq!(epp_key(3), "dev.hwpstate_intel.3.epp");
        let h = FakeHost::new(true);
        assert_eq!(profile(&h), None, "no Speed Shift");
        assert_eq!(ncpu(&h), 1);
        h.int("dev.hwpstate_intel.0.epp", 85).int("hw.ncpu", 8);
        assert_eq!(profile(&h), Some(Profile::LowPower));
        assert_eq!(ncpu(&h), 8);
        let l = FakeHost::new(false);
        assert_eq!(profile(&l), None);
        l.file("/sys/firmware/acpi/platform_profile", "balanced\n");
        assert_eq!(profile(&l), Some(Profile::Automatic));
        l.file("/sys/firmware/acpi/platform_profile", "low-power\n");
        assert_eq!(profile(&l), Some(Profile::LowPower));
    }

    #[test]
    fn power_facts_from_probes() {
        let speed_shift = |argv: &[&str]| match argv.join(" ").as_str() {
            "sysctl -n hw.ncpu" => Some("8\n".to_string()),
            "sysctl -n dev.hwpstate_intel.0.epp" => Some("50\n".to_string()),
            _ => None,
        };
        assert_eq!(
            power_facts(&speed_shift),
            PowerFacts {
                ncpu: 8,
                epp: true,
                powerd: false
            }
        );
        assert_eq!(power_facts(&|_: &[&str]| None), PowerFacts::default());
        let powerd = |argv: &[&str]| (argv == POWERD_ENABLED_ARGV).then(String::new);
        assert_eq!(
            power_facts(&powerd),
            PowerFacts {
                ncpu: 0,
                epp: false,
                powerd: true
            }
        );
        let odd = |argv: &[&str]| (argv == EPP_ARGV).then(|| "unknown oid".to_string());
        assert!(!power_facts(&odd).epp);
    }

    #[test]
    fn lid_switch() {
        let h = FakeHost::new(true);
        assert_eq!(lid(&h), None, "desktop: no lid");
        h.string("hw.acpi.lid_switch_state", "S3");
        assert_eq!(lid(&h), Some(Lid::Sleep));
        h.string("hw.acpi.lid_switch_state", "NONE");
        assert_eq!(lid(&h), Some(Lid::Nothing));
        assert_eq!(lid(&FakeHost::new(false)), None);
        assert_eq!(
            (lid_value(Lid::Sleep), lid_value(Lid::Nothing)),
            ("S3", "NONE")
        );
    }

    #[test]
    fn sysctl_local_upsert() {
        let cur = "# WolOS: no core files\nkern.coredump=0\n hw.acpi.lid_switch_state = NONE \n";
        assert_eq!(
            upsert_sysctl(Some(cur), "hw.acpi.lid_switch_state", "S3"),
            "# WolOS: no core files\nkern.coredump=0\nhw.acpi.lid_switch_state=S3\n"
        );
        assert_eq!(
            upsert_sysctl(Some("kern.coredump=0"), "dev.hwpstate_intel.0.epp", "85"),
            "kern.coredump=0\ndev.hwpstate_intel.0.epp=85\n"
        );
        assert_eq!(
            upsert_sysctl(None, "hw.acpi.lid_switch_state", "S3"),
            "hw.acpi.lid_switch_state=S3\n"
        );
        // A longer key with the same prefix is a different line.
        assert_eq!(
            upsert_sysctl(
                Some("dev.hwpstate_intel.10.epp=0\n"),
                "dev.hwpstate_intel.1.epp",
                "50"
            ),
            "dev.hwpstate_intel.10.epp=0\ndev.hwpstate_intel.1.epp=50\n"
        );
    }

    #[test]
    fn sysctl_allow_list() {
        for (k, v, ok) in [
            ("hw.acpi.lid_switch_state", "S3", true),
            ("hw.acpi.lid_switch_state", "NONE", true),
            ("hw.acpi.lid_switch_state", "S5", false),
            ("hw.acpi.lid_switch_state", "S3;reboot", false),
            ("dev.hwpstate_intel.0.epp", "85", true),
            ("dev.hwpstate_intel.1023.epp", "0", true),
            ("dev.hwpstate_intel.1024.epp", "0", false),
            ("dev.hwpstate_intel.0.epp", "101", false),
            ("dev.hwpstate_intel.0.epp", "-1", false),
            ("dev.hwpstate_intel.0.epp", "+5", false),
            ("dev.hwpstate_intel.0.epp", "", false),
            ("dev.hwpstate_intel.0.epp=1", "1", false),
            ("dev.hwpstate_intel..epp", "1", false),
            ("dev.hwpstate_intel.x.epp", "1", false),
            ("dev.hwpstate_intel.01.epp", "1", false),
            ("kern.securelevel", "-1", false),
            ("-a", "", false),
        ] {
            assert_eq!(is_allowed_sysctl(k, v), ok, "{k}={v}");
        }
    }
}
