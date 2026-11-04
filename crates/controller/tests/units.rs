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
