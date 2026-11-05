// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol doctor hardware` (IMAGE-F, docs/modules/doctor.md): what a modern Intel or AMD laptop
//! needs beyond a driver that loads: hardware video decode, Vulkan, firmware the kernel could
//! not load, a fingerprint reader, Thunderbolt, the processor's power driver, Intel's cooling
//! helper, the power modes, and the x86-64 level against the builds installed. Each row is read
//! through the `Env` (files, listings, commands), so every case here is a fake machine.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use serde_json::Value;
use std::path::PathBuf;

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn rows(env: &mut FakeEnv) -> Vec<Value> {
    env.out.clear();
    run("doctor hardware --json", env);
    let v: Value = serde_json::from_str(&env.out).unwrap_or_else(|e| panic!("{e}: {}", env.out));
    v["checks"].as_array().unwrap().clone()
}

fn row(env: &mut FakeEnv, id: &str) -> Value {
    rows(env)
        .into_iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no check {id}"))
}

fn status(env: &mut FakeEnv, id: &str) -> String {
    row(env, id)["status"].as_str().unwrap().to_owned()
}

fn file(env: &mut FakeEnv, p: &str, text: &str) {
    env.files.push((PathBuf::from(p), text.into()));
}

fn cap(env: &mut FakeEnv, line: &str, out: &str) {
    env.captures.push((line.into(), out.into()));
}

const INTEL_CPU: &str = concat!(
    "processor\t: 0\n",
    "vendor_id\t: GenuineIntel\n",
    "flags\t\t: fpu cx16 lahf_lm popcnt pni sse4_1 sse4_2 ssse3 avx avx2 bmi1 bmi2 f16c fma abm movbe xsave\n",
);
const AMD_CPU: &str = concat!(
    "processor\t: 0\n",
    "vendor_id\t: AuthenticAMD\n",
    "flags\t\t: fpu cx16 lahf_lm popcnt pni sse4_1 sse4_2 ssse3 avx avx2 bmi1 bmi2 f16c fma abm movbe xsave\n",
);
const OLD_CPU: &str = "processor\t: 0\nvendor_id\t: GenuineIntel\nflags\t\t: fpu cx16 lahf_lm popcnt pni sse4_1 sse4_2 ssse3\n";
const VM_CPU: &str = concat!(
    "processor\t: 0\n",
    "vendor_id\t: GenuineIntel\n",
    "flags\t\t: fpu hypervisor cx16 lahf_lm popcnt pni sse4_1 sse4_2 ssse3 avx avx2 bmi1 bmi2 f16c fma abm movbe xsave\n",
);

const VAINFO: &str = concat!(
    "vainfo: VA-API version: 1.22 (libva 2.22.0)\n",
    "vainfo: Driver version: Intel iHD driver for Intel(R) Gen Graphics - 25.2.1 ()\n",
    "vainfo: Supported profile and entrypoints\n",
    "      VAProfileMPEG2Simple            : VAEntrypointVLD\n",
    "      VAProfileH264Main               : VAEntrypointVLD\n",
    "      VAProfileH264Main               : VAEntrypointEncSlice\n",
    "      VAProfileHEVCMain               : VAEntrypointVLD\n",
    "      VAProfileHEVCMain10             : VAEntrypointVLD\n",
    "      VAProfileVP9Profile0            : VAEntrypointVLD\n",
    "      VAProfileAV1Profile0            : VAEntrypointVLD\n",
);
const VAINFO_ENCODE_ONLY: &str = "vainfo: Supported profile and entrypoints\n      VAProfileH264Main               : VAEntrypointEncSlice\n";

const VULKAN_INTEL: &str = concat!(
    "Devices:\n",
    "========\n",
    "GPU0:\n",
    "\tapiVersion         = 1.4.318\n",
    "\tdriverVersion      = 25.2.1\n",
    "\tvendorID           = 0x8086\n",
    "\tdeviceID           = 0x46a6\n",
    "\tdeviceType         = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU\n",
    "\tdeviceName         = Intel(R) Iris(R) Xe Graphics (ADL GT2)\n",
    "\tdriverID           = DRIVER_ID_INTEL_OPEN_SOURCE_MESA\n",
);
const VULKAN_SOFT: &str = concat!(
    "Devices:\n",
    "========\n",
    "GPU0:\n",
    "\tapiVersion         = 1.4.318\n",
    "\tvendorID           = 0x10005\n",
    "\tdeviceType         = PHYSICAL_DEVICE_TYPE_CPU\n",
    "\tdeviceName         = llvmpipe (LLVM 20.1.8, 256 bits)\n",
    "\tdriverID           = DRIVER_ID_MESA_LLVMPIPE\n",
);

fn with_gpu() -> FakeEnv {
    let mut e = FakeEnv::default();
    file(&mut e, "/dev/dri/renderD128", "");
    file(&mut e, "/proc/cpuinfo", INTEL_CPU);
    e
}

#[test]
fn the_area_exists_and_an_empty_machine_skips_every_row() {
    let mut e = FakeEnv::default();
    let all = rows(&mut e);
    let ids: Vec<&str> = all.iter().map(|c| c["id"].as_str().unwrap()).collect();
    for want in [
        "hardware.video",
        "hardware.vulkan",
        "hardware.firmware",
        "hardware.fingerprint",
        "hardware.thunderbolt",
        "hardware.cpu-power",
        "hardware.cooling",
        "hardware.power-modes",
        "hardware.cpu-level",
    ] {
        assert!(ids.contains(&want), "{want} missing from {ids:?}");
    }
    for c in &all {
        assert_eq!(c["area"], "hardware");
        assert_eq!(c["status"], "skipped", "{c}");
    }
}

#[test]
fn video_decode_names_the_codecs_the_graphics_card_decodes() {
    let mut e = with_gpu();
    cap(&mut e, "vainfo", VAINFO);
    let r = row(&mut e, "hardware.video");
    assert_eq!(r["status"], "ok", "{r}");
    let d = r["detail"].as_str().unwrap();
    for codec in ["H.264", "H.265", "VP9", "AV1"] {
        assert!(d.contains(codec), "{d}");
    }
    assert!(!d.contains("vainfo") && !d.contains("VAEntrypoint"), "{d}");
}

#[test]
fn video_that_only_encodes_or_fails_to_start_is_a_warning() {
    let mut e = with_gpu();
    cap(&mut e, "vainfo", VAINFO_ENCODE_ONLY);
    assert_eq!(status(&mut e, "hardware.video"), "warn");
    // vainfo ran but found no driver: scripted as a failed run.
    let mut e = with_gpu();
    e.statuses.push((
        "vainfo".into(),
        1,
        "libva error: vaGetDriverNames() failed".into(),
    ));
    assert_eq!(status(&mut e, "hardware.video"), "warn");
}

#[test]
fn a_virtual_machine_is_not_told_off_for_software_graphics() {
    let mut e = FakeEnv::default();
    file(&mut e, "/dev/dri/renderD128", "");
    file(&mut e, "/proc/cpuinfo", VM_CPU);
    cap(&mut e, "vulkaninfo --summary", VULKAN_SOFT);
    assert_eq!(status(&mut e, "hardware.vulkan"), "skipped");
    assert_eq!(status(&mut e, "hardware.video"), "skipped");
}

#[test]
fn vulkan_runs_on_the_gpu_or_warns_about_the_processor_fallback() {
    let mut e = with_gpu();
    cap(&mut e, "vulkaninfo --summary", VULKAN_INTEL);
    let r = row(&mut e, "hardware.vulkan");
    assert_eq!(r["status"], "ok", "{r}");
    assert!(
        r["detail"]
            .as_str()
            .unwrap()
            .contains("Intel(R) Iris(R) Xe Graphics"),
        "{r}"
    );
    let mut e = with_gpu();
    cap(&mut e, "vulkaninfo --summary", VULKAN_SOFT);
    let r = row(&mut e, "hardware.vulkan");
    assert_eq!(r["status"], "warn", "{r}");
    assert!(r["detail"].as_str().unwrap().contains("processor"), "{r}");
}

#[test]
fn firmware_the_kernel_could_not_load_is_named_by_its_driver() {
    let log = concat!(
        "kernel: iwlwifi 0000:00:14.3: Direct firmware load for iwlwifi-so-a0-gf-a0-89.ucode failed with error -2\n",
        "kernel: iwlwifi 0000:00:14.3: Loaded firmware version: 77.0\n",
        "kernel: bluetooth hci0: Bluetooth: hci0: no suitable firmware found!\n",
        "kernel: amdgpu 0000:03:00.0: Failed to load firmware \"amdgpu/gc_12_0_0_rlc.bin\"\n",
    );
    let mut e = FakeEnv::default();
    cap(&mut e, "journalctl -k -b -q --no-pager", log);
    let r = row(&mut e, "hardware.firmware");
    assert_eq!(r["status"], "warn", "{r}");
    let d = r["detail"].as_str().unwrap();
    assert!(d.contains("amdgpu") && d.contains("bluetooth"), "{d}");
    // The fallback the kernel tries first (-89 before -77) is not a failure.
    assert!(!d.contains("iwlwifi"), "{d}");
    let mut e = FakeEnv::default();
    cap(
        &mut e,
        "journalctl -k -b -q --no-pager",
        "kernel: iwlwifi 0000:00:14.3: Loaded firmware version: 77.0\n",
    );
    assert_eq!(status(&mut e, "hardware.firmware"), "ok");
}

#[test]
fn fingerprint_reader_none_enrolled_enrolled() {
    let mut e = FakeEnv::default();
    e.vars.push(("USER".into(), "ann".into()));
    cap(&mut e, "fprintd-list ann", "No devices available\n");
    assert_eq!(status(&mut e, "hardware.fingerprint"), "skipped");
    cap(
        &mut e,
        "fprintd-list ann",
        concat!(
            "found 1 devices\n",
            "Device at /net/reactivated/Fprint/Device/0\n",
            "Using device /net/reactivated/Fprint/Device/0\n",
            "User ann has no fingers enrolled for Synaptics Sensors.\n",
        ),
    );
    e.captures.remove(0);
    let r = row(&mut e, "hardware.fingerprint");
    assert_eq!(r["status"], "ok", "{r}");
    assert!(
        r["detail"]
            .as_str()
            .unwrap()
            .contains("No finger is added yet"),
        "{r}"
    );
    // Enrolling asks the person to touch the sensor several times: it needs a window of its own,
    // which a captured fix cannot give, so the row offers no button.
    assert!(r.get("fix").is_none() || r["fix"].is_null(), "{r}");
    e.captures.clear();
    cap(
        &mut e,
        "fprintd-list ann",
        concat!(
            "found 1 devices\n",
            "Device at /net/reactivated/Fprint/Device/0\n",
            "Using device /net/reactivated/Fprint/Device/0\n",
            "Fingerprints for user ann on Synaptics Sensors (swipe):\n",
            " - #0: right-index-finger\n",
            " - #1: left-index-finger\n",
        ),
    );
    let r = row(&mut e, "hardware.fingerprint");
    assert_eq!(r["status"], "ok", "{r}");
    assert!(r["detail"].as_str().unwrap().contains("2 fingers"), "{r}");
    assert!(r.get("fix").is_none() || r["fix"].is_null(), "{r}");
}

const BOLT_WAITING: &str = concat!(
    " ● Dell Thunderbolt Dock\n",
    "   ├─ type:          peripheral\n",
    "   ├─ name:          TB16 Dock\n",
    "   ├─ vendor:        Dell\n",
    "   ├─ uuid:          00d4b3a2-0f01-4a5c-8f6e-111122223333\n",
    "   ├─ generation:    Thunderbolt 3\n",
    "   ├─ status:        connected\n",
    "   │  ├─ domain:     domain0\n",
);
const BOLT_OK: &str = concat!(
    " ● Dell Thunderbolt Dock\n",
    "   ├─ type:          peripheral\n",
    "   ├─ uuid:          00d4b3a2-0f01-4a5c-8f6e-111122223333\n",
    "   ├─ status:        authorized\n",
    " ○ Old dock\n",
    "   ├─ uuid:          99999999-0f01-4a5c-8f6e-111122223333\n",
    "   ├─ status:        disconnected\n",
);

#[test]
fn thunderbolt_ports_waiting_devices_and_a_missing_helper() {
    let mut e = FakeEnv::default();
    assert_eq!(status(&mut e, "hardware.thunderbolt"), "skipped");
    // A port, the helper missing: the fix installs it.
    file(
        &mut e,
        "/sys/bus/thunderbolt/devices/domain0/security",
        "user\n",
    );
    let r = row(&mut e, "hardware.thunderbolt");
    assert_eq!(r["status"], "fail", "{r}");
    assert_eq!(r["fix"]["command"], "wol admin pkg-install bolt", "{r}");
    cap(&mut e, "boltctl list", BOLT_WAITING);
    let r = row(&mut e, "hardware.thunderbolt");
    assert_eq!(r["status"], "warn", "{r}");
    assert_eq!(
        r["fix"]["command"], "boltctl enroll 00d4b3a2-0f01-4a5c-8f6e-111122223333",
        "{r}"
    );
    e.captures.clear();
    cap(&mut e, "boltctl list", BOLT_OK);
    let r = row(&mut e, "hardware.thunderbolt");
    assert_eq!(r["status"], "ok", "{r}");
    assert!(r["detail"].as_str().unwrap().contains("1 device"), "{r}");
}

#[test]
fn the_processor_speed_driver_is_modern_or_old() {
    let mut e = FakeEnv::default();
    file(
        &mut e,
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver",
        "intel_pstate\n",
    );
    assert_eq!(status(&mut e, "hardware.cpu-power"), "ok");
    e.files.clear();
    file(
        &mut e,
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver",
        "amd-pstate-epp\n",
    );
    assert_eq!(status(&mut e, "hardware.cpu-power"), "ok");
    e.files.clear();
    file(
        &mut e,
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver",
        "acpi-cpufreq\n",
    );
    let r = row(&mut e, "hardware.cpu-power");
    assert_eq!(r["status"], "warn", "{r}");
    assert!(r["detail"].as_str().unwrap().contains("battery"), "{r}");
}

#[test]
fn intels_cooling_helper_runs_or_has_nothing_to_do() {
    let show = "systemctl show thermald.service -p LoadState -p ActiveState -p ExecMainStatus";
    let mut e = FakeEnv::default();
    file(&mut e, "/proc/cpuinfo", INTEL_CPU);
    cap(
        &mut e,
        show,
        "LoadState=loaded\nActiveState=active\nExecMainStatus=0\n",
    );
    assert_eq!(status(&mut e, "hardware.cooling"), "ok");
    // Ended with status 2: this processor is one it does not manage (the unit calls that success).
    e.captures.clear();
    cap(
        &mut e,
        show,
        "LoadState=loaded\nActiveState=inactive\nExecMainStatus=2\n",
    );
    assert_eq!(status(&mut e, "hardware.cooling"), "ok");
    // Stopped for another reason: warn.
    e.captures.clear();
    cap(
        &mut e,
        show,
        "LoadState=loaded\nActiveState=failed\nExecMainStatus=1\n",
    );
    let r = row(&mut e, "hardware.cooling");
    assert_eq!(r["status"], "warn", "{r}");
    // Root through wol-admind (the cooling-helper action), never a command of its own.
    assert_eq!(r["fix"]["command"], "wol admin cooling-helper on", "{r}");
    assert_eq!(r["fix"]["needs_admin"], true, "{r}");
    // Removed by the person: nothing to look at.
    e.captures.clear();
    cap(
        &mut e,
        show,
        "LoadState=not-found\nActiveState=inactive\nExecMainStatus=0\n",
    );
    assert_eq!(status(&mut e, "hardware.cooling"), "skipped");
    // AMD and virtual machines have no use for it.
    let mut e = FakeEnv::default();
    file(&mut e, "/proc/cpuinfo", AMD_CPU);
    cap(
        &mut e,
        show,
        "LoadState=loaded\nActiveState=failed\nExecMainStatus=1\n",
    );
    assert_eq!(status(&mut e, "hardware.cooling"), "skipped");
    let mut e = FakeEnv::default();
    file(&mut e, "/proc/cpuinfo", VM_CPU);
    cap(
        &mut e,
        show,
        "LoadState=loaded\nActiveState=failed\nExecMainStatus=1\n",
    );
    assert_eq!(status(&mut e, "hardware.cooling"), "skipped");
}

#[test]
fn power_modes_come_from_the_one_power_daemon() {
    let mut e = FakeEnv::default();
    cap(
        &mut e,
        "powerprofilesctl list",
        "  performance:\n    Driver:     intel_pstate\n\n* balanced:\n    Driver:     intel_pstate\n\n  power-saver:\n    Driver:     intel_pstate\n",
    );
    let r = row(&mut e, "hardware.power-modes");
    assert_eq!(r["status"], "ok", "{r}");
    let d = r["detail"].as_str().unwrap();
    assert!(
        d.contains("balanced") && d.contains("power-saver") && d.contains("performance"),
        "{d}"
    );
    let mut e = FakeEnv::default();
    e.statuses
        .push(("powerprofilesctl list".into(), 1, "Couldn't connect".into()));
    assert_eq!(status(&mut e, "hardware.power-modes"), "warn");
}

#[test]
fn the_x86_64_level_agrees_with_the_builds_installed() {
    let off = concat!(
        "[options]\n",
        "Architecture = x86_64 x86_64_v3\n",
        "\n",
        "#[cachyos-v3]\n",
        "#Include = /etc/pacman.d/wolos-cachy-mirrorlist\n",
        "\n",
        "#[cachyos-core-v3]\n",
        "#Include = /etc/pacman.d/wolos-cachy-mirrorlist\n",
        "\n",
        "#[cachyos-extra-v3]\n",
        "#Include = /etc/pacman.d/wolos-cachy-mirrorlist\n",
    );
    let on = off.replace('#', "");
    // A processor that runs v3, builds on or off: fine.
    for conf in [off, on.as_str()] {
        let mut e = FakeEnv::default();
        file(&mut e, "/proc/cpuinfo", INTEL_CPU);
        file(&mut e, "/etc/pacman.conf", conf);
        let r = row(&mut e, "hardware.cpu-level");
        assert_eq!(r["status"], "ok", "{r}");
    }
    // An older processor with the standard builds: fine, and says what that costs (nothing that stops).
    let mut e = FakeEnv::default();
    file(&mut e, "/proc/cpuinfo", OLD_CPU);
    file(&mut e, "/etc/pacman.conf", off);
    let r = row(&mut e, "hardware.cpu-level");
    assert_eq!(r["status"], "ok", "{r}");
    assert!(r["detail"].as_str().unwrap().contains("standard"), "{r}");
    // An older processor with the optimized builds on (a disk moved to another computer): fail.
    let mut e = FakeEnv::default();
    file(&mut e, "/proc/cpuinfo", OLD_CPU);
    file(&mut e, "/etc/pacman.conf", &on);
    let r = row(&mut e, "hardware.cpu-level");
    assert_eq!(r["status"], "fail", "{r}");
    assert!(r["detail"].as_str().unwrap().contains("AVX2"), "{r}");
    assert_eq!(r["fix"]["command"], "wol admin mirror-gate", "{r}");
    assert_eq!(r["fix"]["needs_admin"], true, "{r}");
}

/// UX-LESSONS (tests/doctor.rs): words on screen carry no commands.
#[test]
fn words_on_screen_carry_no_commands() {
    let mut e = with_gpu();
    e.vars.push(("USER".into(), "ann".into()));
    cap(&mut e, "vainfo", VAINFO_ENCODE_ONLY);
    cap(&mut e, "vulkaninfo --summary", VULKAN_SOFT);
    cap(
        &mut e,
        "fprintd-list ann",
        "found 1 devices\nUser ann has no fingers enrolled for X.\n",
    );
    file(
        &mut e,
        "/sys/bus/thunderbolt/devices/domain0/security",
        "user\n",
    );
    cap(&mut e, "boltctl list", BOLT_WAITING);
    file(
        &mut e,
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver",
        "acpi-cpufreq\n",
    );
    for c in rows(&mut e) {
        for k in ["title", "detail"] {
            let t = c[k].as_str().unwrap();
            for bad in [
                "systemctl",
                "pacman",
                "sudo",
                "pkexec",
                "/usr",
                "/etc",
                "/dev",
                "/sys",
                "wol ",
                "--",
                "probably",
                "maybe",
                "might",
                "vainfo",
                "boltctl",
                "fprintd",
            ] {
                assert!(!t.contains(bad), "{} {k}: {t:?} has {bad:?}", c["id"]);
            }
        }
    }
}
