// SPDX-License-Identifier: GPL-2.0-or-later
//! Root unit hardening (audit A7/B5, ADR 0040): every service under files/usr/lib/systemd/system
//! that runs as root (no `User=`) carries the sandbox keys its daemon allows, and the keys it must
//! not have stay out (a mount namespace would make wolos-shm-limit remount a private /dev/shm;
//! ProtectHome would hide the compositor sockets wolos-power talks to). This is the gate until the
//! CI step `systemd-analyze security` (threshold 50) exists. A new root unit fails
//! `every_root_unit_is_hardened` until it has a row here or an EXEMPT reason.
#[path = "support/siblings.rs"]
mod siblings;

use std::path::PathBuf;

const UNITS: &str = "files/usr/lib/systemd/system";

/// (unit, reason): root units deliberately left without sandbox keys.
const EXEMPT: &[(&str, &str)] = &[(
    "wold.service",
    "supervises the login window and every session; any restriction would be inherited by the \
     user's apps (32-bit games, bwrap/Flatpak, setuid helpers). Its defence is kernel identity \
     (B1) and per-job limits",
)];

/// (unit, keys that must be set to exactly this value). Mirrors the `# Hardening` block in
/// each unit, whose comment says what the daemon needs.
const HARDENED: &[(&str, &[&str])] = &[
    (
        "wolos-update-check.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "ProtectSystem=strict",
            "ProtectHome=yes",
            "PrivateTmp=yes",
            "PrivateDevices=yes",
            "ProtectKernelTunables=yes",
            "ProtectKernelModules=yes",
            "ProtectKernelLogs=yes",
            "ProtectControlGroups=yes",
            "ProtectClock=yes",
            "ProtectHostname=yes",
            "RestrictNamespaces=yes",
            "RestrictSUIDSGID=yes",
            "MemoryDenyWriteExecute=yes",
            "SystemCallFilter=@system-service",
            "SystemCallErrorNumber=EPERM",
            "StateDirectory=wolos/updates",
            // pacman -Sy drops to its download user (audit S10), as in wolos-update-download.
            "CapabilityBoundingSet=CAP_SETUID CAP_SETGID CAP_CHOWN CAP_FOWNER CAP_DAC_OVERRIDE CAP_DAC_READ_SEARCH",
            "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
        ],
    ),
    (
        "wolos-report.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "ProtectSystem=strict",
            "ProtectHome=yes",
            "PrivateTmp=yes",
            "PrivateDevices=yes",
            "ProtectKernelTunables=yes",
            "ProtectKernelModules=yes",
            "ProtectKernelLogs=yes",
            "ProtectControlGroups=yes",
            "ProtectClock=yes",
            "ProtectHostname=yes",
            "RestrictNamespaces=yes",
            "RestrictSUIDSGID=yes",
            "MemoryDenyWriteExecute=yes",
            "SystemCallFilter=@system-service",
            "SystemCallErrorNumber=EPERM",
            "StateDirectory=wolos/reports",
            "CapabilityBoundingSet=",
            "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
        ],
    ),
    (
        "wolos-xguard-update.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "ProtectSystem=strict",
            "ProtectHome=yes",
            "PrivateTmp=yes",
            "PrivateDevices=yes",
            "ProtectKernelTunables=yes",
            "ProtectKernelModules=yes",
            "ProtectKernelLogs=yes",
            "ProtectControlGroups=yes",
            "ProtectClock=yes",
            "ProtectHostname=yes",
            "RestrictNamespaces=yes",
            "RestrictSUIDSGID=yes",
            "MemoryDenyWriteExecute=yes",
            "SystemCallFilter=@system-service",
            "SystemCallErrorNumber=EPERM",
            "ReadWritePaths=-/var/lib/wolos/xguard",
            "CapabilityBoundingSet=CAP_CHOWN CAP_FOWNER CAP_DAC_OVERRIDE CAP_DAC_READ_SEARCH",
            "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
        ],
    ),
    (
        "wolos-update-download.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "ProtectSystem=full",
            "ProtectHome=yes",
            "PrivateTmp=yes",
            "ProtectKernelTunables=yes",
            "ProtectKernelModules=yes",
            "ProtectKernelLogs=yes",
            "ProtectControlGroups=yes",
            "ProtectHostname=yes",
            "RestrictSUIDSGID=yes",
            "CapabilityBoundingSet=CAP_SETUID CAP_SETGID CAP_CHOWN CAP_FOWNER CAP_DAC_OVERRIDE CAP_DAC_READ_SEARCH",
            "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK",
        ],
    ),
    (
        "wolos-gpu.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "ProtectSystem=strict",
            "ProtectHome=yes",
            "PrivateTmp=yes",
            "PrivateNetwork=yes",
            "ProtectKernelModules=yes",
            "ProtectKernelLogs=yes",
            "ProtectControlGroups=yes",
            "ProtectClock=yes",
            "ProtectHostname=yes",
            "RestrictNamespaces=yes",
            "RestrictSUIDSGID=yes",
            "MemoryDenyWriteExecute=yes",
            "SystemCallFilter=@system-service",
            "SystemCallErrorNumber=EPERM",
            "CapabilityBoundingSet=",
            "RestrictAddressFamilies=AF_UNIX",
        ],
    ),
    (
        "wolos-mm.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "ProtectSystem=strict",
            "ProtectHome=yes",
            "PrivateTmp=yes",
            "PrivateNetwork=yes",
            "ProtectKernelModules=yes",
            "ProtectKernelLogs=yes",
            "ProtectControlGroups=yes",
            "ProtectClock=yes",
            "ProtectHostname=yes",
            "RestrictNamespaces=yes",
            "RestrictSUIDSGID=yes",
            "MemoryDenyWriteExecute=yes",
            "SystemCallFilter=@system-service",
            "SystemCallErrorNumber=EPERM",
            "CapabilityBoundingSet=",
            "RestrictAddressFamilies=AF_UNIX",
        ],
    ),
    (
        "wolos-boot-ok.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "RestrictSUIDSGID=yes",
            "RestrictNamespaces=yes",
            "ProtectKernelLogs=yes",
            "ProtectHostname=yes",
            "ProtectHome=yes",
            "PrivateNetwork=yes",
            "ProtectKernelModules=yes",
            "RestrictAddressFamilies=AF_UNIX",
        ],
    ),
    (
        "wolos-boot-rearm.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "RestrictSUIDSGID=yes",
            "PrivateNetwork=yes",
            "ProtectHome=yes",
            "ProtectSystem=full",
            "ReadWritePaths=/efi",
            "RestrictAddressFamilies=AF_UNIX",
        ],
    ),
    (
        "wolos-etc-units.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "RestrictSUIDSGID=yes",
            "RestrictNamespaces=yes",
            "RestrictAddressFamilies=AF_UNIX",
            "ProtectSystem=full",
        ],
    ),
    (
        "wolos-power.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "RestrictSUIDSGID=yes",
            "RestrictNamespaces=yes",
            "ProtectKernelLogs=yes",
            "ProtectHostname=yes",
            "ProtectKernelModules=yes",
            "RestrictAddressFamilies=AF_UNIX AF_NETLINK",
        ],
    ),
    (
        "wolos-shm-limit.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "RestrictSUIDSGID=yes",
            "RestrictNamespaces=yes",
            "ProtectKernelLogs=yes",
            "ProtectHostname=yes",
            "PrivateNetwork=yes",
            "ProtectKernelModules=yes",
            "CapabilityBoundingSet=CAP_SYS_ADMIN",
            "RestrictAddressFamilies=AF_UNIX",
        ],
    ),
    (
        "wolos-hw.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "RestrictSUIDSGID=yes",
            "RestrictNamespaces=yes",
            "ProtectKernelLogs=yes",
            "ProtectHostname=yes",
            "ProtectHome=yes",
            "PrivateNetwork=yes",
            "RestrictAddressFamilies=AF_UNIX AF_NETLINK",
        ],
    ),
    (
        "wolos-mok-enroll.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "RestrictSUIDSGID=yes",
            "RestrictNamespaces=yes",
            "ProtectKernelLogs=yes",
            "ProtectHostname=yes",
            "ProtectHome=yes",
            "PrivateTmp=yes",
            "PrivateNetwork=yes",
            "ProtectKernelModules=yes",
            "RestrictAddressFamilies=AF_UNIX",
        ],
    ),
    (
        "wolos-tpm-bind.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "RestrictSUIDSGID=yes",
            "RestrictNamespaces=yes",
            "ProtectKernelLogs=yes",
            "ProtectHostname=yes",
            "ProtectHome=yes",
            "PrivateTmp=yes",
            "PrivateNetwork=yes",
            "ProtectKernelModules=yes",
            "RestrictAddressFamilies=AF_UNIX AF_NETLINK",
        ],
    ),
    (
        "wolos-tpm-reseal.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "RestrictSUIDSGID=yes",
            "RestrictNamespaces=yes",
            "ProtectKernelLogs=yes",
            "ProtectHostname=yes",
            "ProtectHome=yes",
            "PrivateTmp=yes",
            "PrivateNetwork=yes",
            "ProtectKernelModules=yes",
            "RestrictAddressFamilies=AF_UNIX AF_NETLINK",
        ],
    ),
    (
        "wolos-verity-check.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "RestrictSUIDSGID=yes",
            "RestrictNamespaces=yes",
            "ProtectKernelLogs=yes",
            "ProtectHostname=yes",
            "ProtectHome=yes",
            "PrivateNetwork=yes",
            "CapabilityBoundingSet=~CAP_SYS_MODULE",
            "SystemCallFilter=~@module",
            "RestrictAddressFamilies=AF_UNIX",
        ],
    ),
    (
        "wolos-firstboot.service",
        &[
            "NoNewPrivileges=yes",
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "ProtectHome=yes",
            "ProtectKernelLogs=yes",
            "ProtectHostname=yes",
        ],
    ),
    (
        "wolos-xguard.service",
        &[
            "LockPersonality=yes",
            "RestrictRealtime=yes",
            "SystemCallArchitectures=native",
            "RestrictSUIDSGID=yes",
            "RestrictNamespaces=yes",
            "ProtectHostname=yes",
            // S4-3: queue cap vs descriptors, checked in crates/xguard/tests/units.rs.
            "LimitNOFILE=8192",
        ],
    ),
];

/// (unit, keys that must stay out, why).
const FORBIDDEN: &[(&str, &[&str], &str)] = &[
    (
        "wolos-shm-limit.service",
        &[
            "ProtectSystem",
            "ProtectHome",
            "PrivateTmp",
            "PrivateDevices",
            "ReadWritePaths",
        ],
        "a mount namespace would remount a private /dev/shm",
    ),
    (
        "wolos-power.service",
        &["ProtectHome", "ProtectSystem", "ProtectKernelTunables"],
        "talks to /run/user/*/hypr sockets and writes /sys and /etc/wolos",
    ),
    (
        "wolos-power.service",
        &["PrivateNetwork"],
        "iw sets Wi-Fi power saving over nl80211 in the host network namespace (audit CORE2-3)",
    ),
    (
        "wolos-verity-check.service",
        &[
            "ProtectKernelModules",
            "InaccessiblePaths",
            "TemporaryFileSystem",
        ],
        "measures every sealed file, /usr/lib/modules included: a hidden path fails the seal",
    ),
    (
        "wolos-hw.service",
        &[
            "ProtectKernelModules",
            "ProtectKernelTunables",
            "ProtectSystem",
        ],
        "loads drivers and writes /sys and /etc",
    ),
    (
        "wolos-xguard.service",
        &[
            "ProtectSystem",
            "ProtectHome",
            "PrivateDevices",
            "ProtectClock",
            "ProtectKernelModules",
            "ProtectKernelLogs",
        ],
        "fanotify needs the host mount namespace (crates/xguard/tests/units.rs)",
    ),
    (
        "wolos-xguard.service",
        &["MemoryDenyWriteExecute"],
        "the deep scan it forks loads YARA-X rules as machine code (audit S4)",
    ),
    (
        "wold.service",
        &["NoNewPrivileges", "SystemCallFilter", "ProtectHome"],
        "inherited by the whole session",
    ),
];

fn path(unit: &str) -> PathBuf {
    siblings::root().join(UNITS).join(unit)
}

/// `key=value` lines of the [Service] section (comments skipped).
fn service(unit: &str) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(path(unit)).unwrap_or_else(|e| panic!("{unit}: {e}"));
    let mut in_service = false;
    let mut out = Vec::new();
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            in_service = l == "[Service]";
        } else if in_service
            && !l.starts_with('#')
            && let Some((k, v)) = l.split_once('=')
        {
            out.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    out
}

fn root_services() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(siblings::root().join(UNITS))
        .expect("unit directory")
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".service"))
        .filter(|n| !service(n).iter().any(|(k, _)| k == "User"))
        .collect();
    v.sort();
    v
}

#[test]
fn every_root_unit_is_hardened() {
    let roots = root_services();
    assert!(roots.len() >= 13, "found the root units: {roots:?}");
    for u in &roots {
        let listed = HARDENED.iter().any(|(n, _)| n == u) || EXEMPT.iter().any(|(n, _)| n == u);
        assert!(
            listed,
            "{u} runs as root: add a HARDENED row (or an EXEMPT reason)"
        );
    }
    for (u, keys) in HARDENED {
        let have = service(u);
        for want in *keys {
            let (k, v) = want.split_once('=').expect("key=value");
            let got: Vec<&str> = have
                .iter()
                .filter(|(hk, _)| hk == k)
                .map(|(_, hv)| hv.as_str())
                .collect();
            assert_eq!(got, [v], "{u}: {k}");
        }
        for k in [
            "NoNewPrivileges",
            "LockPersonality",
            "RestrictRealtime",
            "SystemCallArchitectures",
        ] {
            assert!(
                have.iter().any(|(hk, _)| hk == k),
                "{u}: {k} is the baseline for every root unit"
            );
        }
        let text = std::fs::read_to_string(path(u)).unwrap_or_default();
        assert!(
            text.contains("# Hardening (audit A7/B5"),
            "{u}: the block says what the daemon needs"
        );
    }
}

#[test]
fn network_fetchers_are_fully_sandboxed() {
    for u in [
        "wolos-update-check.service",
        "wolos-report.service",
        "wolos-xguard-update.service",
    ] {
        let have = service(u);
        let get = |k: &str| {
            have.iter()
                .find(|(hk, _)| hk == k)
                .map(|(_, v)| v.as_str())
                .unwrap_or("")
        };
        assert_eq!(get("ProtectSystem"), "strict", "{u}");
        assert_eq!(get("ProtectHome"), "yes", "{u}");
        assert_eq!(get("SystemCallFilter"), "@system-service", "{u}");
        assert!(
            !get("RestrictAddressFamilies").contains("AF_PACKET"),
            "{u}: no raw sockets"
        );
        assert!(
            !get("CapabilityBoundingSet").contains("CAP_SYS_ADMIN"),
            "{u}: minimal capabilities"
        );
    }
}

#[test]
fn restrictions_a_daemon_cannot_live_with_stay_out() {
    for (u, keys, why) in FORBIDDEN {
        let have = service(u);
        for k in *keys {
            assert!(!have.iter().any(|(hk, _)| hk == k), "{u}: no {k}: {why}");
        }
    }
}

/// The script a unit starts, if it is one of ours (`/usr/lib/wolos/<name>`).
fn script(unit: &str) -> Option<String> {
    let exec = service(unit)
        .into_iter()
        .find(|(k, _)| k == "ExecStart")
        .map(|(_, v)| v)?;
    let name = exec
        .split_whitespace()
        .next()?
        .strip_prefix("/usr/lib/wolos/")?;
    std::fs::read_to_string(siblings::root().join("files/usr/lib/wolos").join(name)).ok()
}

/// pacman 7 downloads as its `DownloadUser`: without CAP_SETUID/CAP_SETGID the switch fails and
/// the check or download quietly finds nothing (audit S10).
#[test]
fn units_that_download_with_pacman_may_switch_to_its_download_user() {
    let mut seen = 0;
    for u in root_services() {
        let Some(text) = script(&u) else { continue };
        let downloads = text.lines().any(|l| {
            let l = l.trim_start();
            !l.starts_with('#')
                && (l.starts_with("checkupdates") || (l.contains("pacman ") && l.contains(" -S")))
        });
        if !downloads {
            continue;
        }
        seen += 1;
        let caps = service(&u)
            .into_iter()
            .find(|(k, _)| k == "CapabilityBoundingSet")
            .map(|(_, v)| v);
        if let Some(caps) = caps {
            for c in ["CAP_SETUID", "CAP_SETGID", "CAP_CHOWN"] {
                assert!(caps.split_whitespace().any(|x| x == c), "{u}: {c}");
            }
        }
    }
    assert!(seen >= 2, "update-check and update-download run pacman");
}

/// Audit S4-26: wold has room for its sockets (soft 4096, above the 1024 that ran out on EMFILE)
/// and keeps the default hard limit; its jobs start again from soft 1024
/// (`api::os::child_nofile`, crates/api/tests/os_procs.rs), so the session inherits nothing.
#[test]
fn wold_has_its_own_descriptor_limit() {
    let have = service("wold.service");
    let got: Vec<&str> = have
        .iter()
        .filter(|(k, _)| k == "LimitNOFILE")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(got, ["4096:524288"]);
}

/// ADR 0012: wol-login (a wold job, root) opens the console VT (`controller::login::CONSOLE`) and
/// locks VT switching (`VT_LOCKSWITCH` needs `CAP_SYS_TTY_CONFIG`). A sandbox key added to
/// wold.service later must not take the device or the capability away.
#[test]
fn wold_lets_the_login_job_reach_the_console_vt() {
    assert_eq!(controller::login::CONSOLE, "/dev/tty1");
    let have = service("wold.service");
    let get = |key: &str| {
        have.iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>()
    };
    assert!(get("User").is_empty(), "wold runs as root");
    assert!(
        get("PrivateDevices").iter().all(|v| *v == "no"),
        "a private /dev has no /dev/tty1"
    );
    let allowed = get("DeviceAllow");
    assert!(
        get("DevicePolicy").iter().all(|v| *v == "auto")
            || allowed
                .iter()
                .any(|a| a.starts_with("/dev/tty1") || a.starts_with("char-tty")),
        "{allowed:?}"
    );
    for v in get("CapabilityBoundingSet") {
        assert!(
            v.split_whitespace().any(|c| c == "CAP_SYS_TTY_CONFIG"),
            "CapabilityBoundingSet={v}"
        );
    }
    let job = std::fs::read_to_string(
        siblings::root().join("desktop/overlay/etc/wolos/jobs/org.wol.login.json"),
    )
    .unwrap();
    let job = shared::jobs::JobSpec::parse(&job).unwrap();
    assert_eq!(job.program, ["/usr/bin/wol-login"]);
    assert_eq!(job.user, None, "the login job keeps wold's root identity");
}

/// F10 (prerelease B6): on a start the firmware did not measure (no event log, PCR 11 never
/// extended), the first-boot bind and the reseal would seal the disk key to values no later start
/// shows, and the next good start asks for the recovery key again. Both wait for a measured start.
#[test]
fn tpm_binding_waits_for_a_start_the_firmware_measured() {
    for u in ["wolos-tpm-bind.service", "wolos-tpm-reseal.service"] {
        let text = std::fs::read_to_string(path(u)).unwrap_or_else(|e| panic!("{u}: {e}"));
        let unit: Vec<&str> = text
            .split("[Service]")
            .next()
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .collect();
        assert!(
            unit.contains(&"ConditionSecurity=measured-uki"),
            "{u}: [Unit] lacks ConditionSecurity=measured-uki"
        );
    }
}

/// QA-PWR-11: the compositor loads `hyprland.conf` while Settings > Keyboard > Shortcuts reads
/// `hyprland.lua` (and `tiling.lua`): two hand-kept copies. Their bind sets must be equal, or the
/// sheet lists keys that do nothing and hides keys that work.
#[test]
fn hypr_conf_and_lua_bind_sets_equal() {
    use wol_core::shortcuts::{Bind, parse, parse_lua};
    const HYPR: &str = "desktop/overlay/usr/share/wolos/hypr";
    let read = |f: &str| {
        std::fs::read_to_string(siblings::root().join(HYPR).join(f))
            .unwrap_or_else(|e| panic!("{f}: {e}"))
    };
    let norm = |binds: Vec<Bind>| {
        let mut v: Vec<String> = binds
            .iter()
            .map(|b| {
                format!(
                    "bind{} {} {} {}",
                    b.flags,
                    b.combo.mods,
                    b.combo.key.to_ascii_lowercase(),
                    b.dispatcher
                ) + &format!(" {}", b.args)
            })
            .collect();
        v.sort();
        v
    };
    for (conf, lua) in [
        ("hyprland.conf", "hyprland.lua"),
        ("tiling.conf", "tiling.lua"),
    ] {
        let (c, l) = (norm(parse(&read(conf))), norm(parse_lua(&read(lua))));
        let only_conf: Vec<&String> = c.iter().filter(|x| !l.contains(x)).collect();
        let only_lua: Vec<&String> = l.iter().filter(|x| !c.contains(x)).collect();
        assert!(
            only_conf.is_empty() && only_lua.is_empty(),
            "{conf} vs {lua}\nonly in .conf: {only_conf:#?}\nonly in .lua: {only_lua:#?}"
        );
        assert!(!c.is_empty(), "{conf}: no binds parsed");
    }
}

/// QA-SYS-8: Arch's bluetooth.service has no Restart=; the drop-in brings bluetoothd back after a
/// crash or a SIGKILL, without a restart storm.
#[test]
fn bluetooth_restart_on_failure() {
    let have = service("bluetooth.service.d/wolos.conf");
    let get = |k: &str| {
        have.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("Restart"), Some("on-failure"));
    let secs: u32 = get("RestartSec")
        .and_then(|v| v.trim_end_matches('s').parse().ok())
        .expect("RestartSec in seconds");
    assert!((1..=10).contains(&secs), "RestartSec={secs}");
    let text = std::fs::read_to_string(path("bluetooth.service.d/wolos.conf")).unwrap();
    assert!(
        text.contains("StartLimitBurst=") && text.contains("StartLimitIntervalSec="),
        "a bluetoothd that dies at once must stop, not loop"
    );
}

/// QA-FILES-10: the login window never turned the display off (no idle handling in login.conf),
/// so a greeter left at the lock screen lit the panel all night. Both compositor files (.conf and
/// .lua) start hypridle on `login-idle.conf`, whose one listener switches the display off at
/// 10 minutes and back on at the first input. The desktop's lock and suspend timers stay out.
#[test]
fn login_conf_has_idle_dpms() {
    const HYPR: &str = "desktop/overlay/usr/share/wolos/hypr";
    let read = |f: &str| {
        std::fs::read_to_string(siblings::root().join(HYPR).join(f))
            .unwrap_or_else(|e| panic!("{f}: {e}"))
    };
    for f in ["login.conf", "login.lua"] {
        let t = read(f);
        assert!(
            t.lines().any(|l| {
                let l = l.trim();
                !l.starts_with(['#', '-'])
                    && l.contains("hypridle -c /usr/share/wolos/hypr/login-idle.conf")
            }),
            "{f} does not start hypridle on login-idle.conf"
        );
    }
    let idle = read("login-idle.conf");
    let code: Vec<&str> = idle
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert!(code.contains(&"timeout = 600"), "{code:?}");
    assert!(
        code.contains(&"on-timeout = hyprctl dispatch dpms off"),
        "{code:?}"
    );
    assert!(
        code.contains(&"on-resume = hyprctl dispatch dpms on"),
        "{code:?}"
    );
    assert!(
        !code
            .iter()
            .any(|l| l.contains("wol-lock") || l.contains("sleep")),
        "the greeter has nobody to lock or suspend: {code:?}"
    );
}

// ---- B6 (hardware-power-image): power, idle, download and malware-engine conditions ----

/// `key=value` lines of one section of any file under the overlaid root (comments skipped).
fn section(rel: &str, name: &str) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(siblings::root().join(rel))
        .unwrap_or_else(|e| panic!("{rel}: {e}"));
    let mut inside = false;
    let mut out = Vec::new();
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            inside = l == format!("[{name}]");
        } else if inside
            && !l.starts_with('#')
            && let Some((k, v)) = l.split_once('=')
        {
            out.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    out
}

/// QA-GFX-9: a USB-C / USB-PD charger plugged or unplugged runs `wol power apply` like a Mains
/// adapter: the rule matches every `type` the policy counts as a charger, and nothing else (a
/// battery's `change` events come every percent).
#[test]
fn udev_rule_matches_usb_pd() {
    let rule = std::fs::read_to_string(
        siblings::root().join("files/usr/lib/udev/rules.d/90-wolos-power.rules"),
    )
    .unwrap();
    let lines: Vec<&str> = rule
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let l = lines[0];
    assert!(
        l.contains(r#"SUBSYSTEM=="power_supply""#) && l.contains(r#"ACTION=="change""#),
        "{l}"
    );
    assert!(l.contains(r#"RUN+="/usr/bin/wol power apply""#), "{l}");
    let types = l
        .split(r#"ATTR{type}==""#)
        .nth(1)
        .and_then(|r| r.split('"').next())
        .expect("ATTR{type}== match");
    let mut got: Vec<&str> = types.split('|').collect();
    let mut want = wol_core::power::policy::CHARGER_TYPES.to_vec();
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(got, want, "udev types vs policy::CHARGER_TYPES");
}

/// The `listener { timeout = N ... }` blocks of a hypridle config: (timeout, block text).
fn listeners(conf: &str) -> Vec<(u32, String)> {
    conf.split("listener")
        .skip(1)
        .filter_map(|b| {
            let t = b
                .lines()
                .find_map(|l| l.trim().strip_prefix("timeout = "))?;
            Some((t.trim().parse().ok()?, b.to_owned()))
        })
        .collect()
}

/// QA-GFX-11: Settings > Lock Screen says the display "turns off one minute after the screen
/// locks". On battery a 120 s listener turned every head off before the 300 s lock (no lock
/// running, the desktop one key away). Whatever the power source, nothing turns the displays off
/// before the lock listener fires; with Never lock, the battery still gets its display off.
#[test]
fn hypridle_battery_dpms_after_lock() {
    let darkens = |b: &str| b.contains("dpms off") || b.contains("wol power idle display-off");
    let check = |what: &str, conf: &str, lock: Option<u32>| {
        let ls = listeners(conf);
        let at = ls
            .iter()
            .find(|(_, b)| b.contains("wol-lock"))
            .map(|(t, _)| *t);
        assert_eq!(at, lock, "{what}: lock listener");
        let off: Vec<u32> = ls
            .iter()
            .filter(|(_, b)| darkens(b))
            .map(|(t, _)| *t)
            .collect();
        assert!(
            !off.is_empty(),
            "{what}: the display turns off at some point"
        );
        if let Some(lock) = lock {
            for t in &off {
                assert!(
                    *t > lock,
                    "{what}: display off at {t} s before the lock at {lock} s"
                );
            }
            assert!(
                off.contains(&(lock + 60)),
                "{what}: one minute after the lock: {off:?}"
            );
        }
    };
    let system = std::fs::read_to_string(
        siblings::root().join("desktop/overlay/usr/share/wolos/hypr/hypridle.conf"),
    )
    .unwrap();
    check("system hypridle.conf", &system, Some(300));
    for secs in [60, 120, 300, 600, 900, 1800] {
        check(
            &format!("Settings, lock {secs}"),
            &wol_core::prefs::hypridle_conf(secs, true),
            Some(secs),
        );
    }
    // Never lock: no lock listener; the battery-only display off still saves the battery.
    let never = wol_core::prefs::hypridle_conf(0, true);
    check("Settings, never", &never, None);
    assert!(never.contains("wol power idle display-off"), "{never}");
    // The page sentence the listeners make true.
    let lock_rs =
        std::fs::read_to_string(siblings::root().join("crates/shell/src/ui/settings/lock.rs"))
            .unwrap();
    assert!(lock_rs.contains("The display turns off one minute after the screen locks."));
}

/// Runs files/usr/lib/wolos/update-download with fake `wol`, `pacman`, `busctl` and `df` (each
/// logs its argv); returns (exit status, log, stderr).
fn download_run(
    name: &str,
    metered: &str,
    free_kb: u64,
    need: u64,
    conf: &str,
) -> (bool, String, String) {
    use std::process::Command;
    let d = std::env::temp_dir().join(format!("wol-units-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let bin = d.join("bin");
    for p in [
        &bin,
        &d.join("db/local"),
        &d.join("db/sync"),
        &d.join("tmp"),
        &d.join("cache"),
    ] {
        std::fs::create_dir_all(p).unwrap();
    }
    let log = d.join("log");
    let fakes = [
        ("wol", String::new()),
        // -Sp prints the size of each package to download (%s), -Sw downloads.
        (
            "pacman",
            format!("case \" $* \" in *\" -Sp \"*) echo {need}; echo 0;; esac"),
        ),
        ("busctl", format!("echo 'u {metered}'")),
        (
            "df",
            format!(
                "echo 'Filesystem 1024-blocks Used Available Capacity Mounted on'; echo \"/dev/x 1 1 {free_kb} 1% /\""
            ),
        ),
    ];
    for (n, body) in fakes {
        let p = bin.join(n);
        std::fs::write(
            &p,
            format!("#!/bin/sh\necho \"{n} $*\" >> {}\n{body}\n", log.display()),
        )
        .unwrap();
        Command::new("chmod").arg("+x").arg(&p).status().unwrap();
    }
    std::fs::write(d.join("auto-update"), conf).unwrap();
    let o = Command::new("sh")
        .arg(siblings::root().join("files/usr/lib/wolos/update-download"))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("WOLOS_AUTO_UPDATE", d.join("auto-update"))
        .env("WOLOS_PACMAN_DB", d.join("db"))
        .env("WOLOS_PACMAN_CACHE", d.join("cache"))
        .env("TMPDIR", d.join("tmp"))
        .output()
        .unwrap();
    let l = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&d);
    (
        o.status.success(),
        l,
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// QA-ACC-18 (ACC C-01): the daily automatic download runs on AC power only (a desktop or VM
/// has no battery and passes), never on a metered connection unless the person said so
/// (`metered=on` in /etc/wolos/auto-update), and only with twice the download free in pacman's
/// cache (the install needs room too). A skipped day is not a failed unit.
#[test]
fn download_timer_conditions() {
    let unit = section(
        "files/usr/lib/systemd/system/wolos-update-download.service",
        "Unit",
    );
    assert!(
        unit.contains(&("ConditionACPower".into(), "true".into())),
        "{unit:?}"
    );
    let downloads = |l: &str| {
        l.lines()
            .any(|x| x.starts_with("pacman ") && x.contains(" -Sw "))
    };
    // NetworkManager's Metered: 1 yes, 3 guess-yes (a phone hotspot) skip; 2 no, 4 guess-no, 0 unknown run.
    for m in ["1", "3"] {
        let (ok, l, err) = download_run(&format!("metered{m}"), m, 10_000_000, 1_000_000, "");
        assert!(ok, "{err}");
        assert!(
            !downloads(&l) && !l.contains("wol update"),
            "metered {m}: {l}"
        );
        assert!(err.contains("metered"), "{err}");
    }
    for m in ["0", "2", "4"] {
        let (ok, l, err) = download_run(&format!("free{m}"), m, 10_000_000, 1_000_000, "");
        assert!(ok && downloads(&l), "metered {m}: {l} {err}");
    }
    // The person allowed metered downloads.
    let (ok, l, _) = download_run(
        "allowed",
        "1",
        10_000_000,
        1_000_000,
        "download=on\nmetered=on\n",
    );
    assert!(ok && downloads(&l), "{l}");
    // 1 GB to download, 1.5 GB free: not enough for download and install.
    let (ok, l, err) = download_run("space", "2", 1_500_000, 1_000_000_000, "");
    assert!(ok, "skipped, not failed: {err}");
    assert!(!downloads(&l), "{l}");
    assert!(
        l.lines()
            .any(|x| x.starts_with("pacman ") && x.contains(" -Sp ")),
        "{l}"
    );
    assert!(err.contains("free"), "{err}");
    // download=off still wins over everything.
    let (ok, l, _) = download_run("off", "2", 10_000_000, 1, "download=off\n");
    assert!(ok && l.is_empty(), "{l}");
}

/// QA-ACC-25 (ACC C-08): the full signature engine (MemoryMax=2G, about 1.2 GB of signatures
/// resident) starts only on machines with the memory for it; below that the socket is not there,
/// so a check uses the quick scanner instead of waiting on an engine that never starts.
#[test]
fn clam_memory_condition() {
    for u in ["wolos-clam.service", "wolos-clam.socket"] {
        let unit = section(&format!("files/usr/lib/systemd/system/{u}"), "Unit");
        assert!(
            unit.contains(&("ConditionMemory".into(), ">=5G".into())),
            "{u}: {unit:?}"
        );
    }
    let s = section("files/usr/lib/systemd/system/wolos-clam.service", "Service");
    assert!(s.contains(&("MemoryMax".into(), "2G".into())), "{s:?}");
}

/// QA-SYS-1, QA-SYS-15, QA-SYS-2: wold is the only way to a screen (no getty, VT locked), so it
/// restarts however often it dies (the default limit, 5 in 10 s, left it failed for good: black
/// screen); the OOM killer picks it last; its jobs outlive it (KillMode=process, `wold` stops them
/// itself on a real stop and leaves them for the next wold on a restart).
#[test]
fn wold_start_limit_zero_and_oom_adjust() {
    let get = |sec: &str, key: &str| {
        section("files/usr/lib/systemd/system/wold.service", sec)
            .into_iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v)
            .collect::<Vec<_>>()
    };
    assert_eq!(get("Unit", "StartLimitIntervalSec"), ["0"]);
    assert!(
        get("Unit", "StartLimitBurst").is_empty(),
        "no burst: no limit"
    );
    assert_eq!(get("Service", "Restart"), ["always"]);
    assert_eq!(get("Service", "OOMScoreAdjust"), ["-900"]);
    assert_eq!(get("Service", "KillMode"), ["process"]);
    // The value is inherited by fork: wold hands every job back to 0 (os.rs), and wold is the
    // only unit that carries it.
    let os = std::fs::read_to_string(siblings::root().join("crates/api/src/os.rs")).unwrap();
    assert!(os.contains("/proc/self/oom_score_adj") && os.contains("fn reset_oom_adj"));
    for u in root_services().iter().filter(|u| *u != "wold.service") {
        assert!(
            !service(u)
                .iter()
                .any(|(k, v)| k == "OOMScoreAdjust" && v.starts_with('-')),
            "{u}: an inheriting root unit must not shield its children from the OOM killer"
        );
    }
}

/// QA-PRIV-14: wold.service has no sandbox keys, on purpose and written down. Every key below is
/// inherited by what wold starts: the whole user session (apps, games, `sudo` in a terminal, bwrap
/// and Flatpak), root's admin daemon (pacman, modprobe, mount, chroot for slots) and the login
/// window. A mount namespace, a seccomp filter or a smaller capability bounding set on wold
/// binds all of them. The `# No sandbox keys` comment names the decision; this list keeps a key
/// from arriving without somebody reading why.
#[test]
fn wold_hardening_lines() {
    const INHERITED: &[&str] = &[
        "NoNewPrivileges",
        "PrivateTmp",
        "PrivateDevices",
        "PrivateNetwork",
        "PrivateUsers",
        "PrivateMounts",
        "ProtectSystem",
        "ProtectHome",
        "ProtectKernelTunables",
        "ProtectKernelModules",
        "ProtectKernelLogs",
        "ProtectControlGroups",
        "ProtectClock",
        "ProtectHostname",
        "ProtectProc",
        "ReadOnlyPaths",
        "InaccessiblePaths",
        "RestrictNamespaces",
        "RestrictSUIDSGID",
        "RestrictRealtime",
        "RestrictAddressFamilies",
        "SystemCallFilter",
        "SystemCallArchitectures",
        "MemoryDenyWriteExecute",
        "LockPersonality",
        "CapabilityBoundingSet",
        "DevicePolicy",
        "LimitCORE",
    ];
    let have = service("wold.service");
    for (k, v) in &have {
        assert!(
            !INHERITED.contains(&k.as_str()),
            "wold.service: {k}={v} would bind the session, sudo and pacman it starts"
        );
    }
    let text = std::fs::read_to_string(path("wold.service")).unwrap();
    assert!(text.contains("QA-PRIV-14") && text.contains("No sandbox keys"));
    assert!(EXEMPT.iter().any(|(u, _)| *u == "wold.service"));
}

/// QA L2-30: the user's whole slice has a memory ceiling, so the kernel's out-of-memory pick
/// happens inside it (the hog, not logind/seatd/wold which live outside); the ceiling sits above
/// the user manager's own `MemoryMax=80%` (background services) and under the machine's RAM.
#[test]
fn user_slice_memory_ceiling_keeps_the_oom_pick_inside_the_session() {
    let slice = section(
        "files/usr/lib/systemd/system/user-.slice.d/70-wolos-limits.conf",
        "Slice",
    );
    let max: Vec<&str> = slice
        .iter()
        .filter(|(k, _)| k == "MemoryMax")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(max, ["90%"]);
    let mgr = section(
        "files/usr/lib/systemd/system/user@.service.d/70-wolos-limits.conf",
        "Service",
    );
    assert!(mgr.iter().any(|(k, v)| k == "MemoryMax" && v == "80%"));
}

// ---- B5 boot rows (layers-seal-boot) ----

/// `key=value` lines of any section of a unit (comments skipped).
fn unit_lines(unit: &str) -> Vec<String> {
    std::fs::read_to_string(path(unit))
        .unwrap_or_else(|e| panic!("{unit}: {e}"))
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && l.contains('='))
        .map(str::to_owned)
        .collect()
}

/// QA-LIVE3-2: the seal check measures the slot as sealed, before systemd-sysext merges the
/// layers over /usr (through the merge every file read as changed and every boot fell back).
#[test]
fn verity_check_before_sysext() {
    let u = unit_lines("wolos-verity-check.service");
    assert!(
        u.iter().any(|l| l == "Before=systemd-sysext.service"),
        "{u:?}"
    );
    // D3: a deleted manifest must not skip the check (`wol seal check` passes an unsealed slot).
    assert!(
        !u.iter().any(|l| l.starts_with("ConditionPathExists=")),
        "{u:?}"
    );
    assert!(u.iter().any(|l| l == "DefaultDependencies=no"), "{u:?}");
}

/// QA-LIVE3-36: boot-ok reads the marks the early checks leave, so it runs after them; it runs
/// on every good boot (a blessed entry pulls no boot-complete.target, QA-LIVE4-4) and has time
/// for its 90 s wait for the login window (QA-SYS-3).
#[test]
fn boot_ok_requires_seal_check() {
    let u = unit_lines("wolos-boot-ok.service");
    for want in [
        "After=wolos-verity-check.service",
        "After=wold.service",
        "Before=boot-complete.target",
        "RequiredBy=boot-complete.target",
        "WantedBy=multi-user.target",
        "TimeoutStartSec=4min",
    ] {
        assert!(
            u.iter().any(|l| l == want),
            "wolos-boot-ok lacks {want}: {u:?}"
        );
    }
    let script =
        std::fs::read_to_string(siblings::root().join("files/usr/lib/wolos/boot-ok")).unwrap();
    assert!(
        script.contains("verity-failed epoch-failed"),
        "boot-ok reads both marks"
    );
}

/// QA-SET-R2-01 / QA-LIVE3-4: the /etc overlay mounts after systemd read /etc, so a unit turned
/// on since the install was never part of the boot. Once the overlay is up, systemd reads the
/// units again (only when the overlay holds unit changes) and the default target takes in what
/// that adds, before sysinit; the unit is on by preset.
#[test]
fn etc_overlay_before_systemd_reads_etc() {
    let u = unit_lines("wolos-etc-units.service");
    for want in [
        "DefaultDependencies=no",
        "After=etc.mount local-fs.target",
        "Before=sysinit.target",
        "ConditionPathIsMountPoint=/etc",
        // D6: a hostname-only change triggers too (either condition, OR-ed).
        "ConditionDirectoryNotEmpty=|/var/lib/wolos/etc/upper/systemd",
        "ConditionPathExists=|/var/lib/wolos/etc/upper/hostname",
        "ExecStart=/usr/lib/wolos/etc-units",
        "WantedBy=sysinit.target",
    ] {
        assert!(
            u.iter().any(|l| l == want),
            "wolos-etc-units lacks {want}: {u:?}"
        );
    }
    let script =
        std::fs::read_to_string(siblings::root().join("files/usr/lib/wolos/etc-units")).unwrap();
    let reload = script.find("systemctl daemon-reload").unwrap();
    let start = script
        .find("systemctl start --no-block default.target")
        .unwrap();
    assert!(reload < start, "read the units, then take them in");
    let preset = std::fs::read_to_string(
        siblings::root().join("files/usr/lib/systemd/system-preset/50-wolos.preset"),
    )
    .unwrap();
    assert!(
        preset
            .lines()
            .any(|l| l == "enable wolos-etc-units.service")
    );
    // The overlay it waits for is the one install-run writes.
    let run =
        std::fs::read_to_string(siblings::root().join("files/usr/lib/wolos/install-run")).unwrap();
    assert!(
        run.contains("overlay /etc overlay lowerdir=/etc,upperdir=/var/lib/wolos/etc/upper,"),
        "install-run's /etc overlay line"
    );
}

/// QA-PERF-3 (H17-03): GTK 4 registers every app with the portal at display open and waits for
/// `xdg-desktop-portal` to activate its backends (2.8 s warm, 10 s at first boot). The session
/// starts the portal and the accessibility bus in the background before the shell, so the wait is
/// over by the time the shell asks. Both files the compositor can load carry the line, ahead of
/// `wol-shell`, and without blocking the compositor's startup.
#[test]
fn portal_started_before_the_shell() {
    const HYPR: &str = "desktop/overlay/usr/share/wolos/hypr";
    for file in ["hyprland.conf", "hyprland.lua"] {
        let text = std::fs::read_to_string(siblings::root().join(HYPR).join(file)).unwrap();
        let at = |needle: &str| {
            text.lines()
                .position(|l| l.contains(needle) && !l.trim_start().starts_with(['#', '-']))
                .unwrap_or_else(|| panic!("{file}: no line with {needle}"))
        };
        let portal = at("systemctl --user start --no-block xdg-desktop-portal.service");
        let line = text.lines().nth(portal).unwrap();
        for unit in ["xdg-desktop-portal-gtk.service", "at-spi-dbus-bus.service"] {
            assert!(line.contains(unit), "{file}: {unit}");
        }
        assert!(
            portal < at("wol-shell --supervised"),
            "{file}: the portal first"
        );
        // The user manager has the display and D-Bus environment before it starts the portal.
        assert!(
            at("dbus-update-activation-environment --systemd") < portal,
            "{file}: environment first"
        );
    }
}

/// live-4 F3: the shutdown unit that counts the running slot's entry again runs its script at
/// stop, while /efi and /var are still mounted, on every installed boot (preset).
#[test]
fn shutdown_rearm_unit() {
    let u = unit_lines("wolos-boot-rearm.service");
    for want in [
        "RequiresMountsFor=/efi /var",
        "RemainAfterExit=yes",
        "ExecStart=/usr/lib/wolos/boot-rearm start",
        "ExecStop=/usr/lib/wolos/boot-rearm stop",
        "WantedBy=multi-user.target",
    ] {
        assert!(
            u.iter().any(|l| l == want),
            "wolos-boot-rearm lacks {want}: {u:?}"
        );
    }
    let preset = std::fs::read_to_string(
        siblings::root().join("files/usr/lib/systemd/system-preset/50-wolos.preset"),
    )
    .unwrap();
    assert!(
        preset
            .lines()
            .any(|l| l == "enable wolos-boot-rearm.service")
    );
}

/// Audit8 stability2 #5: a oneshot has no start timeout of its own, so a stalled fetch would hold
/// wolos-update-check.service and the hourly timer would never start it again until a restart.
/// The unit is bounded and so is every fetch in its script.
#[test]
fn the_update_check_never_hangs_for_good() {
    let have = service("wolos-update-check.service");
    let got: Vec<&str> = have
        .iter()
        .filter(|(k, _)| k == "TimeoutStartSec")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(got, ["30min"]);
    let text = script("wolos-update-check.service").expect("update-check");
    let fetches: Vec<&str> = text
        .lines()
        .map(str::trim_start)
        .filter(|l| !l.starts_with('#') && (l.contains("curl ") || l.contains("get ")))
        .collect();
    // The script fetches through one `get` helper (audit 8 #5) that holds the only `curl`: it is
    // called for the manifest and its signature, and its `curl` line is checked below.
    assert!(fetches.len() >= 3, "{fetches:?}");
    for l in &text
        .lines()
        .map(str::trim_start)
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
    {
        for (i, _) in l.match_indices("curl ") {
            let call = l[i..].split('|').next().unwrap_or_default();
            assert!(call.contains("--max-time "), "unbounded fetch: {l}");
        }
    }
}

/// Audit8 stability2 #6: the queued-modules drain runs as one transient unit name
/// (`wolos-modules-pending`); a stall must not hold that name for good, or no later connection
/// drains the queue. Both starters bound the unit (SIGINT first: pacman stops between packages),
/// and modules-install bounds its install step as well as its download.
#[test]
fn the_queued_modules_drain_is_bounded() {
    for f in [
        "files/usr/lib/wolos/firstboot",
        "desktop/overlay/etc/NetworkManager/dispatcher.d/50-wolos-online",
    ] {
        let text = std::fs::read_to_string(siblings::root().join(f)).unwrap();
        // The systemd-run call, its continuation lines joined.
        let call = text
            .split("systemd-run ")
            .nth(1)
            .and_then(|r| r.split("modules-install").next())
            .unwrap_or_else(|| panic!("{f}: no systemd-run"));
        assert!(call.contains("--unit=wolos-modules-pending"), "{f}: {call}");
        assert!(call.contains("--property=RuntimeMaxSec="), "{f}: {call}");
        assert!(call.contains("--property=KillSignal=SIGINT"), "{f}: {call}");
    }
    let text =
        std::fs::read_to_string(siblings::root().join("files/usr/lib/wolos/modules-install"))
            .unwrap();
    let installs: Vec<&str> = text
        .lines()
        .map(str::trim_start)
        .filter(|l| !l.starts_with('#') && l.contains("pacman \"-S$y\""))
        .collect();
    assert_eq!(installs.len(), 2, "{installs:?}");
    for l in installs {
        assert!(l.starts_with("timeout -s INT -k 60 \"$isecs\" "), "{l}");
    }
}

fn repo_text(p: &str) -> String {
    std::fs::read_to_string(siblings::root().join(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// QA-PRIV-13 (B3): sudo and sudo -i count wrong passwords with pam_faillock on the one count
/// the lock screen and Settings use (10 failures lock for 60 s), not Arch system-auth's own.
#[test]
fn sudo_has_faillock() {
    for f in ["sudo", "sudo-i"] {
        let pam = repo_text(&format!("desktop/overlay/etc/pam.d/{f}"));
        let auth: Vec<&str> = pam.lines().filter(|l| l.starts_with("auth")).collect();
        for stage in ["preauth", "authfail"] {
            assert!(
                auth.iter().any(|l| l.contains("pam_faillock.so")
                    && l.contains(stage)
                    && l.contains("deny=10")
                    && l.contains("unlock_time=60")),
                "{f}: no pam_faillock {stage}: {pam}"
            );
        }
        assert!(auth.iter().any(|l| l.contains("pam_unix.so")), "{f}: {pam}");
        assert!(!auth.iter().any(|l| l.contains("system-")), "{f}: {pam}");
        assert!(
            pam.lines()
                .any(|l| l.starts_with("account") && l.contains("pam_faillock.so")),
            "{f}: failures never reset: {pam}"
        );
    }
}

/// QA-STAB-8 (B3): lock on sleep holds the system's sleep (logind delay inhibitor) until the
/// session is locked, in the shipped default and in what Settings writes.
#[test]
fn hypridle_sleep_inhibitor() {
    let shipped = repo_text("desktop/overlay/usr/share/wolos/hypr/hypridle.conf");
    for c in [shipped.clone(), wol_core::prefs::hypridle_conf(300, true)] {
        let general = c.split("listener").next().unwrap();
        assert!(
            general.contains("before_sleep_cmd = ") && general.contains("wol-lock"),
            "{c}"
        );
        assert!(general.contains("\n    inhibit_sleep = 3\n"), "{c}");
    }
}

/// QA-LIVE3-24 (B3): a refused sudo never insults the person (Arch's sudo is built with insults).
#[test]
fn sudoers_no_insults() {
    let s = repo_text("desktop/overlay/etc/sudoers.d/wolos");
    let lines: Vec<&str> = s
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .collect();
    assert!(lines.contains(&"Defaults !insults"), "{s}");
    assert!(
        !lines
            .iter()
            .any(|l| l.starts_with("Defaults") && l.contains(" insults")),
        "{s}"
    );
}

/// QA-CRASH-5 (B3): the session helpers hypridle (lock on idle and before sleep), hyprsunset,
/// wol-realtime, the polkit agent and the warm Search are user units that come back after a crash, part of
/// wolos-session.target, which the compositor's start lines (re)start; the portal is the package's
/// own unit, started with the other portals. No compositor start line runs one of them bare.
#[test]
fn session_helpers_have_restart() {
    let dir = "files/usr/lib/systemd/user";
    let target = repo_text(&format!("{dir}/wolos-session.target"));
    for (unit, prog) in [
        ("wolos-hypridle.service", "hypridle"),
        ("wolos-hyprsunset.service", "hyprsunset"),
        ("wolos-realtime.service", "wol-realtime"),
        ("wolos-polkit-agent.service", "lxqt-policykit-agent"),
        // QA-LN-3: the warm Search, kept by the session the same way.
        ("wolos-search.service", "wol-search --warm"),
    ] {
        let u = repo_text(&format!("{dir}/{unit}"));
        let lines: Vec<&str> = u.lines().map(str::trim).collect();
        for want in ["Restart=on-failure", "PartOf=wolos-session.target"] {
            assert!(lines.contains(&want), "{unit} lacks {want}: {u}");
        }
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("ExecStart=") && l.contains(prog)),
            "{unit}: {u}"
        );
        assert!(
            target.contains(unit),
            "wolos-session.target does not want {unit}"
        );
        for conf in ["hyprland.lua", "hyprland.conf"] {
            let c = repo_text(&format!("desktop/overlay/usr/share/wolos/hypr/{conf}"));
            let bare = c
                .lines()
                .filter(|l| l.contains("exec-once") || l.starts_with("once("))
                .any(|l| {
                    l.contains(&format!("exec {prog}"))
                        || l.contains(&format!("\"{prog}\""))
                        || l.trim_end().ends_with(&format!("= {prog}"))
                });
            assert!(!bare, "{conf} still starts {prog} with no restart");
        }
    }
    for conf in ["hyprland.lua", "hyprland.conf"] {
        let c = repo_text(&format!("desktop/overlay/usr/share/wolos/hypr/{conf}"));
        assert!(
            c.contains("systemctl --user restart wolos-session.target"),
            "{conf} never starts the session helpers"
        );
        assert!(
            !c.contains("/usr/lib/xdg-desktop-portal-hyprland\""),
            "{conf}: bare portal"
        );
    }
}

/// QA-LIVE-41 (B3): the lock screen's supervisor is a user unit that comes back when it is killed
/// (both wol-lock processes killed left a black screen with no way back), never after a real unlock
/// (exit 0) nor on a compositor with no session lock (69, shell lock.rs LOCK_UNSUPPORTED).
#[test]
fn wold_restarts_wol_lock_when_session_locked() {
    let u = repo_text("files/usr/lib/systemd/user/wolos-lock.service");
    let lines: Vec<&str> = u.lines().map(str::trim).collect();
    for want in [
        "ExecStart=/usr/bin/wol-lock",
        "Restart=on-failure",
        "RestartPreventExitStatus=69",
        "PartOf=wolos-session.target",
    ] {
        assert!(
            lines.contains(&want),
            "wolos-lock.service lacks {want}: {u}"
        );
    }
    let lock = repo_text("crates/shell/src/ui/lock.rs");
    assert!(lock.contains("pub const LOCK_UNIT: &str = \"wolos-lock.service\";"));
    assert!(lock.contains("pub const LOCK_UNSUPPORTED: i32 = 69;"));
}

/// QA-LIVE-38 (B3): the polkit password dialog stays on top and keeps the keyboard until it is
/// answered, so a password is never typed into the terminal behind it.
#[test]
fn polkit_agent_stays_focused() {
    let conf = repo_text("desktop/overlay/usr/share/wolos/hypr/hyprland.conf");
    let block = conf
        .split("windowrule {")
        .find(|b| b.contains("name = wol-polkit-agent"))
        .expect("hyprland.conf: no wol-polkit-agent rule");
    let lua = repo_text("desktop/overlay/usr/share/wolos/hypr/hyprland.lua");
    let rule = &lua[lua
        .find("name = \"wol-polkit-agent\"")
        .expect("hyprland.lua: no rule")..];
    let rule = &rule[..rule.find("})").unwrap()];
    for key in ["pin", "stay_focused", "dim_around", "center", "float"] {
        assert!(
            block.contains(&format!("    {key} = on")),
            "conf lacks {key}: {block}"
        );
        assert!(
            rule.contains(&format!("{key} = true")),
            "lua lacks {key}: {rule}"
        );
    }
    assert!(
        block.contains("match:class = ^(lxqt-policykit-agent)$"),
        "{block}"
    );
    assert!(
        rule.contains("class = \"^(lxqt-policykit-agent)$\""),
        "{rule}"
    );
    let agent = repo_text("files/usr/lib/systemd/user/wolos-polkit-agent.service");
    assert!(agent.contains("ExecStart=/usr/bin/lxqt-policykit-agent"));
}
