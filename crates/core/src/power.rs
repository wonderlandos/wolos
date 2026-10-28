//! Battery details (`/sys/class/power_supply`), energy profile (platform profile / cpufreq EPP)
//! and lid action (systemd-logind) for Settings > Battery.
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

/// Full / design capacity as a percentage (capped at 100).
fn health(full: Option<u32>, design: Option<u32>) -> Option<u8> {
    let (f, d) = (full?, design?);
    (d > 0).then(|| (u64::from(f) * 100 / u64::from(d)).min(100) as u8)
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

/// Battery-type supplies of the computer (not a mouse's: scope Device), sorted.
pub fn batteries(h: &dyn Host) -> Vec<String> {
    h.list_dir("/sys/class/power_supply")
        .into_iter()
        .filter(|d| {
            h.read_text(&format!("{d}/type"))
                .is_some_and(|t| t.trim() == "Battery")
                && h.read_text(&format!("{d}/scope"))
                    .is_none_or(|s| s.trim() != "Device")
        })
        .collect()
}

/// First battery, None without one.
pub fn battery(h: &dyn Host) -> Option<BatteryInfo> {
    let dir = batteries(h).into_iter().next()?;
    parse_sysfs(|k| h.read_text(&format!("{dir}/{k}")))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Profile {
    LowPower,
    Automatic,
    Performance,
}

pub const PLATFORM_PROFILE: &str = "/sys/firmware/acpi/platform_profile";
pub const EPP_CPU0: &str = "/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference";

/// ACPI platform profile, power-profiles-daemon or cpufreq EPP name -> profile.
pub fn profile_of(name: &str) -> Option<Profile> {
    match name.trim() {
        "low-power" | "quiet" | "cool" | "power-saver" | "power" | "balance_power" => {
            Some(Profile::LowPower)
        }
        "balanced" | "balance_performance" | "default" => Some(Profile::Automatic),
        "performance" | "balanced-performance" => Some(Profile::Performance),
        _ => None,
    }
}

/// Current profile: the firmware platform profile, else the CPU energy preference (intel_pstate,
/// amd-pstate-epp). None: nothing to steer.
pub fn profile(h: &dyn Host) -> Option<Profile> {
    h.read_text(PLATFORM_PROFILE)
        .or_else(|| h.read_text(EPP_CPU0))
        .and_then(|p| profile_of(&p))
}

/// Online CPUs (`/sys/devices/system/cpu/online`); 1 when unknown.
pub fn ncpu(h: &dyn Host) -> u32 {
    h.read_text("/sys/devices/system/cpu/online")
        .map(|t| parse_cpu_list(&t))
        .filter(|n| *n > 0)
        .unwrap_or(1)
}

/// Kernel cpu list ("0-3,6,8-9") -> count.
pub fn parse_cpu_list(t: &str) -> u32 {
    t.trim()
        .split(',')
        .filter_map(|r| match r.split_once('-') {
            Some((a, b)) => Some(b.parse::<u32>().ok()?.checked_sub(a.parse().ok()?)? + 1),
            None => r.parse::<u32>().ok().map(|_| 1),
        })
        .sum()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Lid {
    Sleep,
    Nothing,
}

/// logind drop-in wol-admind `lid` writes (systemd-logind rereads `logind.conf.d` on HUP).
pub const LID_CONF: &str = "/etc/systemd/logind.conf.d/50-wolos-lid.conf";
/// Make logind reread its configuration.
pub const LOGIND_RELOAD_ARGV: [&str; 5] = ["systemctl", "kill", "-s", "HUP", "systemd-logind"];

/// `HandleLidSwitch` value.
pub fn lid_value(l: Lid) -> &'static str {
    match l {
        Lid::Sleep => "suspend",
        Lid::Nothing => "ignore",
    }
}

/// The whole [`LID_CONF`] file: the same action on battery and on AC.
pub fn lid_conf(l: Lid) -> String {
    format!(
        "# Written by WolOS (Settings > Battery).\n[Login]\nHandleLidSwitch={v}\nHandleLidSwitchExternalPower={v}\n",
        v = lid_value(l)
    )
}

/// `HandleLidSwitch` in a logind config (last wins); None when unset.
pub fn parse_lid_conf(text: &str) -> Option<Lid> {
    let v = text.lines().rev().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "HandleLidSwitch").then(|| v.trim())
    })?;
    Some(if v == "ignore" {
        Lid::Nothing
    } else {
        Lid::Sleep
    })
}

/// Lid action; None without a lid (desktops). logind suspends by default.
pub fn lid(h: &dyn Host) -> Option<Lid> {
    if h.list_dir("/proc/acpi/button/lid").is_empty() {
        return None;
    }
    Some(
        h.read_text(LID_CONF)
            .and_then(|t| parse_lid_conf(&t))
            .unwrap_or(Lid::Sleep),
    )
}

/// sysctl.d drop-in for settings wol-admind may change.
pub const SYSCTL_LOCAL: &str = "/etc/sysctl.d/90-wolos-local.conf";

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

/// wol-admind allow-list for [`SYSCTL_LOCAL`]. Linux: nothing yet (the lid is logind's, the
/// energy profile power-profiles-daemon's).
pub fn is_allowed_sysctl(_key: &str, _value: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

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
    fn battery_is_the_first_system_battery() {
        let l = FakeHost::linux();
        l.file("/sys/class/power_supply/ADP0/type", "Mains\n")
            .file("/sys/class/power_supply/ADP0/online", "1\n")
            // A Bluetooth mouse: scope Device, not the computer's battery.
            .file("/sys/class/power_supply/hidpp_battery_0/type", "Battery\n")
            .file("/sys/class/power_supply/hidpp_battery_0/scope", "Device\n")
            .file("/sys/class/power_supply/hidpp_battery_0/capacity", "90\n")
            .file("/sys/class/power_supply/BAT0/type", "Battery\n")
            .file("/sys/class/power_supply/BAT0/capacity", "40\n")
            .file("/sys/class/power_supply/BAT0/status", "Discharging\n");
        assert_eq!(
            battery(&l).map(|b| (b.percent, b.charge)),
            Some((40, Charge::Discharging))
        );
        assert_eq!(battery(&FakeHost::linux()), None);
    }

    #[test]
    fn energy_profile() {
        let l = FakeHost::linux();
        assert_eq!(profile(&l), None);
        l.file(EPP_CPU0, "balance_power\n");
        assert_eq!(profile(&l), Some(Profile::LowPower));
        l.file(PLATFORM_PROFILE, "balanced\n");
        assert_eq!(
            profile(&l),
            Some(Profile::Automatic),
            "platform profile first"
        );
        l.file(PLATFORM_PROFILE, "performance\n");
        assert_eq!(profile(&l), Some(Profile::Performance));
        assert_eq!(profile_of("bogus"), None);
        assert_eq!(ncpu(&l), 1);
        l.file("/sys/devices/system/cpu/online", "0-7\n");
        assert_eq!(ncpu(&l), 8);
        assert_eq!(parse_cpu_list("0-3,6,8-9\n"), 7);
        assert_eq!(parse_cpu_list("0"), 1);
        assert_eq!(parse_cpu_list("junk"), 0);
    }

    #[test]
    fn lid_is_logind() {
        let h = FakeHost::linux();
        assert_eq!(lid(&h), None, "desktop: no lid");
        h.file("/proc/acpi/button/lid/LID0/state", "state:      open\n");
        assert_eq!(lid(&h), Some(Lid::Sleep), "logind default: suspend");
        h.file(LID_CONF, &lid_conf(Lid::Nothing));
        assert_eq!(lid(&h), Some(Lid::Nothing));
        assert_eq!(parse_lid_conf(&lid_conf(Lid::Sleep)), Some(Lid::Sleep));
        assert_eq!(parse_lid_conf("[Login]\n"), None);
        assert_eq!(
            lid_conf(Lid::Nothing),
            "# Written by WolOS (Settings > Battery).\n[Login]\nHandleLidSwitch=ignore\nHandleLidSwitchExternalPower=ignore\n"
        );
    }

    #[test]
    fn sysctl_local_upsert() {
        let cur = "# WolOS\nkernel.kptr_restrict=2\n vm.swappiness = 10 \n";
        assert_eq!(
            upsert_sysctl(Some(cur), "vm.swappiness", "60"),
            "# WolOS\nkernel.kptr_restrict=2\nvm.swappiness=60\n"
        );
        assert_eq!(upsert_sysctl(None, "a.b", "1"), "a.b=1\n");
        assert_eq!(
            upsert_sysctl(Some("a.b10=0\n"), "a.b1", "5"),
            "a.b10=0\na.b1=5\n"
        );
        assert!(!is_allowed_sysctl("kernel.kptr_restrict", "0"));
    }

    #[test]
    fn real_host_power_reads() {
        let h = crate::system::PosixHost;
        assert!(ncpu(&h) >= 1);
        // Whatever this machine has must parse: a battery has a percentage, a lid an action.
        if let Some(b) = battery(&h) {
            assert!(b.percent <= 100);
        }
        if !h.list_dir("/proc/acpi/button/lid").is_empty() {
            assert!(lid(&h).is_some());
        }
    }
}
