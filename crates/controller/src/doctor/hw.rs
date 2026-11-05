// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol doctor hardware` (docs/modules/doctor.md, IMAGE-F): what a modern Intel or AMD laptop
//! needs beyond a driver that loads. The `display` and `network` areas answer "has it a driver";
//! these rows answer "does the part work as well as the hardware can": hardware video decode
//! (VA-API), Vulkan on the GPU and not the processor, firmware the kernel could not load, a
//! fingerprint reader, Thunderbolt, the processor's power driver, Intel's cooling helper, the
//! power modes, and the x86-64 level against the builds installed (ADR 0030).
//!
//! Every row reads the machine through [`Env`] (files, listings, commands), so a fake machine
//! drives each case. The parsers are pure and have their own tests below. A row whose tool
//! cannot run, or whose part is absent, is `Skipped`, never a failure: nobody has to own a
//! fingerprint reader or a dock.
use super::{Check, Fix, Status, check, plural, req};
use crate::run::Env;
use shared::admin::AdminAction;
use std::path::Path;
use wol_core::mirror as m;

const AREA: &str = "hardware";

/// All rows, in the order the Doctor app lists them.
pub fn checks(env: &mut dyn Env) -> Vec<Check> {
    let cpu = env
        .read_file(Path::new("/proc/cpuinfo"))
        .unwrap_or_default();
    let vm = is_vm(&cpu);
    let gpu = render_node(env);
    vec![
        video(env, gpu, vm),
        vulkan(env, gpu, vm),
        firmware(env),
        fingerprint(env),
        thunderbolt(env),
        cpu_power(env),
        cooling(env, &cpu, vm),
        power_modes(env),
        cpu_level(env, &cpu),
    ]
}

// ---------------------------------------------------------------- pure parsers

/// A virtual machine (the CPU flags say so): software graphics and no thermal chip are normal there.
pub fn is_vm(cpuinfo: &str) -> bool {
    cpuinfo.lines().any(|l| {
        l.split_once(':').is_some_and(|(k, v)| {
            k.trim() == "flags" && v.split_whitespace().any(|f| f == "hypervisor")
        })
    })
}

/// The codecs `vainfo` lists with a decode entry point (`VAEntrypointVLD`), in the order people
/// know them. Encode-only profiles do not count: playback needs decode.
pub fn va_decode(vainfo: &str) -> Vec<&'static str> {
    const FAMILIES: [(&str, &str); 4] = [
        ("VAProfileH264", "H.264"),
        ("VAProfileHEVC", "H.265"),
        ("VAProfileVP9", "VP9"),
        ("VAProfileAV1", "AV1"),
    ];
    let mut v: Vec<&'static str> = Vec::new();
    for l in vainfo.lines() {
        let Some((profile, entry)) = l.split_once(':') else {
            continue;
        };
        if entry.trim() != "VAEntrypointVLD" {
            continue;
        }
        let profile = profile.trim();
        if let Some((_, name)) = FAMILIES.iter().find(|(p, _)| profile.starts_with(p))
            && !v.contains(name)
        {
            v.push(name);
        }
    }
    v.sort_by_key(|n| FAMILIES.iter().position(|(_, f)| f == n));
    v
}

/// One Vulkan device of `vulkaninfo --summary`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VkDevice {
    pub name: String,
    /// `PHYSICAL_DEVICE_TYPE_CPU` is the software renderer (llvmpipe).
    pub software: bool,
}

pub fn vulkan_devices(summary: &str) -> Vec<VkDevice> {
    let mut v = Vec::new();
    let mut software = false;
    for l in summary.lines() {
        let Some((k, val)) = l.split_once('=') else {
            continue;
        };
        match k.trim() {
            "deviceType" => software = val.trim() == "PHYSICAL_DEVICE_TYPE_CPU",
            "deviceName" => v.push(VkDevice {
                name: val.trim().into(),
                software,
            }),
            _ => {}
        }
    }
    v
}

/// Drivers whose kernel log says a firmware load ended in failure, not the first tries of a
/// fallback list (iwlwifi asks for `-89.ucode`, then `-88`..., and loads `-77`: `Direct firmware
/// load ... failed with error -2` is normal noise there). Only the final verdicts count.
pub fn firmware_failures(kernel_log: &str) -> Vec<String> {
    const FINAL: [&str; 6] = [
        "no suitable firmware found",
        "failed to load firmware",
        "firmware load failed",
        "firmware loading failed",
        "failed to request firmware",
        "firmware file not found",
    ];
    let mut v: Vec<String> = Vec::new();
    for l in kernel_log.lines() {
        let low = l.to_lowercase();
        if !FINAL.iter().any(|f| low.contains(f)) {
            continue;
        }
        // `kernel: <driver> <device>: message` or `<driver> <device>: ...` or `<subsystem> hci0: ...`.
        let body = l.split_once("kernel:").map_or(l, |(_, r)| r).trim();
        let driver = body.split([' ', ':']).next().unwrap_or("").to_lowercase();
        if !driver.is_empty() && !v.contains(&driver) {
            v.push(driver);
        }
    }
    v
}

/// What `fprintd-list` said.
#[derive(Debug, PartialEq, Eq)]
pub enum Reader {
    /// "No devices available".
    None,
    /// A reader, with this many fingers enrolled for the account.
    Found(usize),
}

pub fn fprintd_list(text: &str) -> Option<Reader> {
    if text.contains("No devices available") {
        return Some(Reader::None);
    }
    if !text.contains("found ") || !text.contains("devices") {
        return None;
    }
    Some(Reader::Found(
        text.lines()
            .filter(|l| l.trim_start().starts_with("- #"))
            .count(),
    ))
}

/// A Thunderbolt/USB4 device from `boltctl list`.
#[derive(Debug, PartialEq, Eq)]
pub struct BoltDevice {
    pub uuid: String,
    pub status: String,
}

pub fn bolt_devices(list: &str) -> Vec<BoltDevice> {
    let mut v: Vec<BoltDevice> = Vec::new();
    let mut uuid = String::new();
    for l in list.lines() {
        let t = l
            .trim_start_matches([' ', '│', '├', '└', '─', '●', '○'])
            .trim();
        if let Some(u) = t.strip_prefix("uuid:") {
            uuid = u.trim().into();
        } else if let Some(s) = t.strip_prefix("status:")
            && !uuid.is_empty()
        {
            v.push(BoltDevice {
                uuid: std::mem::take(&mut uuid),
                status: s.trim().into(),
            });
        }
    }
    v
}

fn bolt_authorized(status: &str) -> bool {
    status.starts_with("authorized")
}

/// Waiting to be allowed: plugged in (`connected`) or a failed attempt, not merely remembered.
fn bolt_waiting(status: &str) -> bool {
    matches!(status, "connected" | "auth-error")
}

/// Power modes of `powerprofilesctl list`, in the daemon's order.
pub fn power_profiles(list: &str) -> Vec<String> {
    list.lines()
        .filter_map(|l| {
            let t = l.trim_start_matches([' ', '*']);
            let name = t.strip_suffix(':')?;
            (!name.is_empty() && !name.contains(' ') && !l.starts_with("    "))
                .then(|| name.to_owned())
        })
        .collect()
}

/// `systemctl show -p LoadState -p ActiveState -p ExecMainStatus`: (load, active, exit status of
/// the main process; -1 when unknown).
fn unit_state(text: &str) -> (String, String, i32) {
    let get = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('=')))
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    (
        get("LoadState"),
        get("ActiveState"),
        get("ExecMainStatus").parse().unwrap_or(-1),
    )
}

fn names(list: &[impl AsRef<str>]) -> String {
    let l: Vec<&str> = list.iter().map(AsRef::as_ref).collect();
    match l.as_slice() {
        [] => String::new(),
        [a] => (*a).into(),
        [a, b] => format!("{a} and {b}"),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

// ---------------------------------------------------------------- rows

/// A render node means a GPU the session can use.
fn render_node(env: &dyn Env) -> bool {
    env.list_dir(Path::new("/dev/dri"))
        .iter()
        .any(|p| p.contains("renderD"))
}

fn video(env: &mut dyn Env, gpu: bool, vm: bool) -> Check {
    let t = "Video decoding";
    if !gpu || vm {
        return check(
            AREA,
            "video",
            t,
            Status::Skipped,
            "There is no graphics card to decode video on.",
        );
    }
    match env.capture_status(&["vainfo"]) {
        None => check(
            AREA,
            "video",
            t,
            Status::Skipped,
            "Video decoding could not be checked.",
        ),
        Some((0, out, _)) => {
            let codecs = va_decode(&out);
            if codecs.is_empty() {
                check(
                    AREA,
                    "video",
                    t,
                    Status::Warn,
                    "The graphics card does not decode video, so films play on the processor and use more battery.",
                )
            } else {
                check(
                    AREA,
                    "video",
                    t,
                    Status::Ok,
                    &format!(
                        "The graphics card decodes {} video, so films use little battery.",
                        names(&codecs)
                    ),
                )
            }
        }
        Some(_) => check(
            AREA,
            "video",
            t,
            Status::Warn,
            "The video driver of the graphics card did not start, so films play on the processor and use more battery.",
        ),
    }
}

fn vulkan(env: &mut dyn Env, gpu: bool, vm: bool) -> Check {
    let t = "3D graphics";
    if !gpu || vm {
        return check(
            AREA,
            "vulkan",
            t,
            Status::Skipped,
            "There is no graphics card for 3D games.",
        );
    }
    match env.capture_status(&["vulkaninfo", "--summary"]) {
        None => check(
            AREA,
            "vulkan",
            t,
            Status::Skipped,
            "3D graphics could not be checked.",
        ),
        Some((0, out, _)) => {
            let devs = vulkan_devices(&out);
            let real: Vec<&str> = devs
                .iter()
                .filter(|d| !d.software)
                .map(|d| d.name.as_str())
                .collect();
            if !real.is_empty() {
                check(
                    AREA,
                    "vulkan",
                    t,
                    Status::Ok,
                    &format!("3D games and apps run on {}.", names(&real)),
                )
            } else {
                check(
                    AREA,
                    "vulkan",
                    t,
                    Status::Warn,
                    "3D games and apps run on the processor, which is slow: the graphics card has no 3D driver.",
                )
            }
        }
        Some(_) => check(
            AREA,
            "vulkan",
            t,
            Status::Warn,
            "3D games and apps run on the processor, which is slow: the 3D driver did not start.",
        ),
    }
}

fn firmware(env: &mut dyn Env) -> Check {
    let t = "Device firmware";
    let Some(log) = env.capture(&["journalctl", "-k", "-b", "-q", "--no-pager"]) else {
        return check(
            AREA,
            "firmware",
            t,
            Status::Skipped,
            "The startup messages could not be read.",
        );
    };
    let failed = firmware_failures(&log);
    if failed.is_empty() {
        check(
            AREA,
            "firmware",
            t,
            Status::Ok,
            "Every part found the firmware it asked for at startup.",
        )
    } else {
        check(
            AREA,
            "firmware",
            t,
            Status::Warn,
            &format!(
                "{} could not load firmware at startup. A system update usually brings it.",
                names(&failed)
            ),
        )
    }
}

fn fingerprint(env: &mut dyn Env) -> Check {
    let t = "Fingerprint reader";
    let user = env
        .var("USER")
        .or_else(|| env.var("LOGNAME"))
        .or_else(|| env.capture(&["id", "-un"]).map(|u| u.trim().to_owned()))
        .filter(|u| !u.is_empty());
    let Some(user) = user else {
        return check(
            AREA,
            "fingerprint",
            t,
            Status::Skipped,
            "Fingerprint readers are checked from a signed-in account.",
        );
    };
    let Some((_, out, err)) = env.capture_status(&["fprintd-list", &user]) else {
        return check(
            AREA,
            "fingerprint",
            t,
            Status::Skipped,
            "Fingerprint readers could not be checked.",
        );
    };
    match fprintd_list(&format!("{out}\n{err}")) {
        Some(Reader::None) => check(
            AREA,
            "fingerprint",
            t,
            Status::Skipped,
            "No fingerprint reader was found.",
        ),
        Some(Reader::Found(0)) => check(
            AREA,
            "fingerprint",
            t,
            Status::Ok,
            "A fingerprint reader works. No finger is added yet.",
        ),
        Some(Reader::Found(n)) => check(
            AREA,
            "fingerprint",
            t,
            Status::Ok,
            &format!(
                "A fingerprint reader works, with {n} {} added.",
                plural(n, "finger", "fingers")
            ),
        ),
        None => check(
            AREA,
            "fingerprint",
            t,
            Status::Skipped,
            "Fingerprint readers could not be checked.",
        ),
    }
}

fn thunderbolt(env: &mut dyn Env) -> Check {
    let t = "Thunderbolt and USB4";
    let has_port = env
        .list_dir(Path::new("/sys/bus/thunderbolt/devices"))
        .iter()
        .any(|p| {
            p.rsplit('/')
                .next()
                .is_some_and(|n| n.starts_with("domain"))
        });
    if !has_port {
        return check(
            AREA,
            "thunderbolt",
            t,
            Status::Skipped,
            "This computer has no Thunderbolt or USB4 port.",
        );
    }
    let Some((code, out, _)) = env.capture_status(&["boltctl", "list"]) else {
        return check(
            AREA,
            "thunderbolt",
            t,
            Status::Fail,
            "This computer has a Thunderbolt port, but the helper that allows docks and drives is missing.",
        )
        .with(Fix::admin("Install the Thunderbolt helper", vec![req(AdminAction::PkgInstall, &["bolt"])]));
    };
    if code != 0 {
        return check(
            AREA,
            "thunderbolt",
            t,
            Status::Warn,
            "The Thunderbolt service is not answering.",
        );
    }
    let devs = bolt_devices(&out);
    let waiting: Vec<&BoltDevice> = devs.iter().filter(|d| bolt_waiting(&d.status)).collect();
    if let Some(first) = waiting.first() {
        let n = waiting.len();
        return check(
            AREA,
            "thunderbolt",
            t,
            Status::Warn,
            &format!(
                "{n} Thunderbolt {} plugged in and waiting for you to allow {}.",
                plural(n, "device is", "devices are"),
                plural(n, "it", "them")
            ),
        )
        .with(Fix::user(
            "Allow the device",
            ["boltctl", "enroll", first.uuid.as_str()]
                .map(String::from)
                .to_vec(),
        ));
    }
    let here = devs.iter().filter(|d| bolt_authorized(&d.status)).count();
    check(
        AREA,
        "thunderbolt",
        t,
        Status::Ok,
        &if here == 0 {
            "Thunderbolt works. Nothing is plugged in.".to_owned()
        } else {
            format!(
                "Thunderbolt works. {here} {} connected.",
                plural(here, "device", "devices")
            )
        },
    )
}

fn cpu_power(env: &mut dyn Env) -> Check {
    let t = "Processor power";
    let Some(driver) = env
        .read_file(Path::new(
            "/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver",
        ))
        .map(|d| d.trim().to_owned())
        .filter(|d| !d.is_empty())
    else {
        return check(
            AREA,
            "cpu-power",
            t,
            Status::Skipped,
            "This processor does not report its speed control.",
        );
    };
    match driver.as_str() {
        "acpi-cpufreq" | "pcc-cpufreq" | "powernow-k8" | "speedstep-centrino" => check(
            AREA,
            "cpu-power",
            t,
            Status::Warn,
            "The processor uses an older speed control, so the battery lasts less and the fan runs more. A newer processor has the modern one.",
        ),
        _ => check(
            AREA,
            "cpu-power",
            t,
            Status::Ok,
            "The processor manages its own speed with the modern control, which saves battery.",
        ),
    }
}

fn cooling(env: &mut dyn Env, cpu: &str, vm: bool) -> Check {
    let t = "Cooling";
    let intel = cpu.lines().any(|l| l.contains("GenuineIntel"));
    if !intel || vm {
        return check(
            AREA,
            "cooling",
            t,
            Status::Skipped,
            "Intel's cooling helper is only for Intel processors in a real computer.",
        );
    }
    let Some(out) = env.capture(&[
        "systemctl",
        "show",
        "thermald.service",
        "-p",
        "LoadState",
        "-p",
        "ActiveState",
        "-p",
        "ExecMainStatus",
    ]) else {
        return check(
            AREA,
            "cooling",
            t,
            Status::Skipped,
            "The cooling helper could not be checked.",
        );
    };
    match unit_state(&out) {
        (l, ..) if l == "not-found" => check(
            AREA,
            "cooling",
            t,
            Status::Skipped,
            "Intel's cooling helper is not installed on this computer.",
        ),
        (_, s, _) if s == "active" => check(
            AREA,
            "cooling",
            t,
            Status::Ok,
            "Intel's cooling helper is running.",
        ),
        // thermald ends with status 2 on a processor it has no table for; the unit calls that success.
        (_, _, 2) => check(
            AREA,
            "cooling",
            t,
            Status::Ok,
            "This Intel processor manages its own cooling, so the helper has nothing to do.",
        ),
        _ => check(
            AREA,
            "cooling",
            t,
            Status::Warn,
            "Intel's cooling helper is not running, so the fan may start later than it should.",
        )
        .with(Fix::admin(
            "Turn the helper on",
            vec![req(AdminAction::CoolingHelper, &["on"])],
        )),
    }
}

fn power_modes(env: &mut dyn Env) -> Check {
    let t = "Power modes";
    let Some((code, out, _)) = env.capture_status(&["powerprofilesctl", "list"]) else {
        return check(
            AREA,
            "power-modes",
            t,
            Status::Skipped,
            "Power modes could not be checked.",
        );
    };
    if code != 0 {
        return check(
            AREA,
            "power-modes",
            t,
            Status::Warn,
            "The power mode service is not answering, so Energy Mode cannot change anything.",
        );
    }
    let modes = power_profiles(&out);
    if modes.is_empty() {
        return check(
            AREA,
            "power-modes",
            t,
            Status::Warn,
            "The power mode service lists no modes.",
        );
    }
    check(
        AREA,
        "power-modes",
        t,
        Status::Ok,
        &format!("Energy Mode can use {}.", names(&modes)),
    )
}

fn cpu_level(env: &mut dyn Env, cpu: &str) -> Check {
    let t = "Processor level";
    let level = m::x86_64_level(cpu);
    if level == 0 {
        return check(
            AREA,
            "cpu-level",
            t,
            Status::Skipped,
            "The processor level could not be read.",
        );
    }
    let conf = env.read_file(Path::new(m::PACMAN_CONF)).unwrap_or_default();
    let optimized = m::cachy_enabled(&conf);
    if level >= m::CACHY_LEVEL {
        return check(
            AREA,
            "cpu-level",
            t,
            Status::Ok,
            if optimized {
                "This processor runs the optimized programs, and they are on."
            } else {
                "This processor can run the optimized programs; they turn on with the next update check."
            },
        );
    }
    if optimized {
        return check(
            AREA,
            "cpu-level",
            t,
            Status::Fail,
            "The optimized programs are on, but this processor lacks the AVX2 they need. Some programs will stop with an error.",
        )
        .with(Fix::admin(
            "Go back to the standard programs",
            vec![req(AdminAction::MirrorGate, &[])],
        ));
    }
    check(
        AREA,
        "cpu-level",
        t,
        Status::Ok,
        "This processor lacks AVX2, so it uses the standard programs. Everything works; a few run a little slower.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_counts_vld_only_in_the_known_order() {
        let t = concat!(
            "VAProfileAV1Profile0 : VAEntrypointVLD\n",
            "VAProfileH264Main : VAEntrypointEncSlice\n",
            "VAProfileVP9Profile2 : VAEntrypointVLD\n",
            "VAProfileHEVCMain10 : VAEntrypointVLD\n",
            "VAProfileHEVCMain : VAEntrypointVLD\n",
            "VAProfileMPEG2Main : VAEntrypointVLD\n",
        );
        assert_eq!(va_decode(t), ["H.265", "VP9", "AV1"]);
        assert!(va_decode("").is_empty());
    }

    #[test]
    fn vulkan_devices_pair_type_and_name() {
        let t = concat!(
            "GPU0:\n",
            "\tdeviceType = PHYSICAL_DEVICE_TYPE_DISCRETE_GPU\n",
            "\tdeviceName = AMD Radeon RX 7600 (RADV NAVI33)\n",
            "GPU1:\n",
            "\tdeviceType = PHYSICAL_DEVICE_TYPE_CPU\n",
            "\tdeviceName = llvmpipe\n",
        );
        assert_eq!(
            vulkan_devices(t),
            [
                VkDevice {
                    name: "AMD Radeon RX 7600 (RADV NAVI33)".into(),
                    software: false
                },
                VkDevice {
                    name: "llvmpipe".into(),
                    software: true
                },
            ]
        );
    }

    #[test]
    fn firmware_failures_skip_the_fallback_list() {
        let t = concat!(
            "iwlwifi 0000:00:14.3: Direct firmware load for iwlwifi-so-a0-gf-a0-89.ucode failed with error -2\n",
            "kernel: btusb 1-6:1.0: firmware load failed: -2\n",
            "kernel: btusb 1-6:1.0: firmware load failed: -2\n",
        );
        assert_eq!(firmware_failures(t), ["btusb"]);
    }

    #[test]
    fn fingerprints_counted_by_their_entries() {
        assert_eq!(fprintd_list("No devices available"), Some(Reader::None));
        assert_eq!(
            fprintd_list("found 1 devices\nUser ann has no fingers enrolled for X.\n"),
            Some(Reader::Found(0))
        );
        assert_eq!(
            fprintd_list("found 1 devices\n - #0: right-index-finger\n - #1: left-thumb\n"),
            Some(Reader::Found(2))
        );
        assert_eq!(fprintd_list("Failed to connect"), None);
    }

    #[test]
    fn bolt_statuses_are_read_per_uuid() {
        let t = concat!(
            " ● A\n",
            "   ├─ uuid:          u1\n",
            "   ├─ status:        authorized-secure\n",
            " ○ B\n",
            "   ├─ uuid:          u2\n",
            "   ├─ status:        disconnected\n",
            " ● C\n",
            "   ├─ uuid:          u3\n",
            "   ├─ status:        connected\n",
        );
        let d = bolt_devices(t);
        assert_eq!(d.len(), 3);
        assert!(bolt_authorized(&d[0].status) && !bolt_waiting(&d[0].status));
        assert!(!bolt_authorized(&d[1].status) && !bolt_waiting(&d[1].status));
        assert!(bolt_waiting(&d[2].status) && d[2].uuid == "u3");
    }

    #[test]
    fn profiles_and_unit_state() {
        assert_eq!(
            power_profiles("  performance:\n    Driver:     x\n\n* balanced:\n    Driver:     x\n"),
            ["performance", "balanced"]
        );
        assert_eq!(
            unit_state("LoadState=loaded\nActiveState=inactive\nExecMainStatus=2\n"),
            ("loaded".to_owned(), "inactive".to_owned(), 2)
        );
        assert_eq!(unit_state(""), (String::new(), String::new(), -1));
        assert_eq!(names(&["a", "b", "c"]), "a, b and c");
    }
}
