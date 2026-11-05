// SPDX-License-Identifier: GPL-2.0-or-later
//! The systemd user units of a Tor group, as `systemd-run` argv (ADR 0053 D2, D4).
//!
//! Two transient services per group, so tor has a network and the browser does not:
//! - `wol-tor-<g>.service` runs `wol-tor tor-run` (writes the torrc, seeds the cache, then
//!   becomes tor) in a tight sandbox with a tmpfs `RuntimeDirectory`;
//! - `atlas-tor-<g>.service` runs `wol-tor host` (netns check, relay, then Atlas) with
//!   `PrivateNetwork=yes`, an empty home, the Tor policies bound over the normal ones, no swap
//!   and no core files.
//!
//! The property lists are data so a test can read them back; the host re-checks the namespace at
//! run time anyway ([`crate::netns`]), because systemd may skip `PrivateNetwork=` silently.
use crate::group::{Group, Paths};
use crate::profile;
use std::path::Path;

/// Where Atlas reads managed policies (ADR 0043, transform T-04); the Atlas package ships the
/// (empty) directory, because `ProtectSystem=strict` leaves systemd no place to create it.
pub const POLICY_TARGET: &str = "/etc/atlas/policies";
/// The relay's address inside the browser's namespace (and the Tor policy's proxy).
pub const RELAY_ADDR: &str = "127.0.0.1:9150";

/// Unit name of the group's tor.
pub fn tor_unit_name(g: &Group) -> String {
    format!("wol-tor-{g}.service")
}

/// Unit name of the group's Atlas instance.
pub fn host_unit_name(g: &Group) -> String {
    format!("atlas-tor-{g}.service")
}

/// Sandbox shared by tor and the cache run (they need the network, nothing else).
fn tor_sandbox() -> Vec<String> {
    [
        "NoNewPrivileges=yes",
        "PrivateUsers=yes",
        "PrivateTmp=yes",
        "PrivateDevices=yes",
        "ProtectSystem=strict",
        "ProtectHome=tmpfs",
        "ProtectKernelTunables=yes",
        "ProtectKernelModules=yes",
        "ProtectKernelLogs=yes",
        "ProtectControlGroups=yes",
        "ProtectClock=yes",
        "ProtectHostname=yes",
        "ProtectProc=invisible",
        "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
        "RestrictNamespaces=yes",
        "RestrictRealtime=yes",
        "RestrictSUIDSGID=yes",
        "LockPersonality=yes",
        "MemoryDenyWriteExecute=yes",
        "SystemCallArchitectures=native",
        "SystemCallFilter=@system-service",
        "SystemCallFilter=~@privileged @resources",
        "CapabilityBoundingSet=",
        "UMask=0077",
        "LimitCORE=0",
        "MemorySwapMax=0",
        "StandardOutput=null",
        "StandardError=null",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn run_prefix(unit: &str, props: &[String]) -> Vec<String> {
    let mut v: Vec<String> = ["systemd-run", "--user", "--collect", "--quiet"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    v.push(format!("--unit={unit}"));
    for p in props {
        v.push(format!("--property={p}"));
    }
    v.push("--".into());
    v
}

/// `systemd-run` argv that starts the group's tor.
pub fn tor_unit(g: &Group, wol_tor: &Path) -> Vec<String> {
    let mut props = tor_sandbox();
    props.push(format!("RuntimeDirectory={}", Paths::runtime_directory(g)));
    props.push("RuntimeDirectoryMode=0700".into());
    // tor exits 0 when its owner (the host) goes away: that is not a failure, so no restart.
    props.push("Restart=on-failure".into());
    props.push("RestartSec=1".into());
    props.push(format!("Description=Tor for the Atlas Tor group {g}"));
    let mut v = run_prefix(&tor_unit_name(g), &props);
    v.extend([
        wol_tor.display().to_string(),
        "tor-run".into(),
        "--group".into(),
        g.to_string(),
    ]);
    v
}

/// Sandbox of the browser's unit: no network but loopback, no home, nothing kept. `paths` names
/// the launcher-written profile (bound read-only; if it is missing the unit fails, which is the
/// intended fail-closed) and the unit's own tmpfs runtime directory.
pub fn host_properties(g: &Group, paths: &Paths) -> Vec<String> {
    let mut p: Vec<String> = [
        "PrivateNetwork=yes",
        "PrivateUsers=yes",
        "PrivateTmp=yes",
        "ProtectHome=tmpfs",
        "ProtectSystem=strict",
        "NoNewPrivileges=yes",
        "UMask=0077",
        "LimitCORE=0",
        "MemorySwapMax=0",
        "RuntimeDirectoryMode=0700",
        "Environment=TZ=UTC",
        "Environment=LANG=en_US.UTF-8",
        "Environment=LC_ALL=en_US.UTF-8",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    p.push(format!(
        "RuntimeDirectory={}",
        Paths::browser_runtime_directory(g)
    ));
    p.push(format!(
        "Environment=FONTCONFIG_FILE={}",
        paths.profile.join(profile::FONTS_FILE).display()
    ));
    // No `-` prefix on purpose: a missing source fails the unit, so Atlas never runs without
    // the Tor policies.
    p.push(format!(
        "BindReadOnlyPaths={}:{POLICY_TARGET}",
        paths.profile.join(profile::POLICY_SUBDIR).display()
    ));
    p.push(format!("Description=Atlas Tor group {g}"));
    p
}

/// `systemd-run` argv that starts the group's Atlas instance; `pass_env` names the session
/// variables the browser needs (Wayland, D-Bus, runtime dir), taken from the caller.
pub fn host_unit(
    g: &Group,
    paths: &Paths,
    wol_tor: &Path,
    atlas: &Path,
    pass_env: &[&str],
    shared_tor: Option<&Path>,
) -> Vec<String> {
    let mut props = host_properties(g, paths);
    // Wants, not BindsTo: when tor stops the browser stays and loads nothing (the kill switch,
    // journey J-T3); when the host ends, tor exits because the host owned its control connection.
    // With the machine scope on (`shared_tor`), the group has no tor of its own: it would be
    // redirected into the machine's tor, a needless second hop, so the relay uses the machine
    // tor's SOCKS socket (isolation by credential, as ever) and owns nothing.
    if shared_tor.is_none() {
        props.push(format!("Wants={}", tor_unit_name(g)));
        props.push(format!("After={}", tor_unit_name(g)));
    }
    let mut v = run_prefix(&host_unit_name(g), &props);
    // --setenv=NAME copies NAME from the caller; inserted before "--".
    let dashdash = v.len() - 1;
    for (i, e) in pass_env.iter().enumerate() {
        v.insert(dashdash + i, format!("--setenv={e}"));
    }
    v.extend([
        wol_tor.display().to_string(),
        "host".into(),
        "--group".into(),
        g.to_string(),
    ]);
    if let Some(sock) = shared_tor {
        v.extend(["--tor-socks".into(), sock.display().to_string()]);
    }
    v.extend(["--".into(), atlas.display().to_string()]);
    v
}

/// The system unit of the machine-wide tor (ADR 0056), as file text: tor as its own user, the
/// sandbox of every tor unit, listeners on the loopback and the app bridge. The privileged daemon
/// installs it; it is `Wants=`-ed by nothing, started only by the plan of [`crate::mode`].
pub fn gateway_unit_file(wol_tor: &Path, tor_user: &str) -> String {
    let mut s = String::from(concat!(
        "# Generated by wol-tor (ADR 0056). Started and stopped by wol-admind.\n",
        "[Unit]\nDescription=Tor for the machine and for chosen apps\n",
        "After=network-online.target\n\n[Service]\n"
    ));
    s.push_str(&format!(
        "User={tor_user}\nExecStart={} tor-run --gateway\n",
        wol_tor.display()
    ));
    // RuntimeDirectory: sockets and cookie. StateDirectory (/var/lib/wol-tor/gateway): tor's
    // DataDirectory, so the guards, the consensus and the circuit-build timeout survive restarts
    // (a machine-wide tor with a fresh guard at every boot is worse than one with a kept guard).
    s.push_str("RuntimeDirectory=wol-tor/gateway\nRuntimeDirectoryMode=0700\n");
    s.push_str("StateDirectory=wol-tor/gateway\nStateDirectoryMode=0700\nRestart=on-failure\nRestartSec=1\n");
    for p in tor_sandbox() {
        // A system service as its own user needs neither a user namespace nor a tmpfs home.
        let p = match p.as_str() {
            "PrivateUsers=yes" => continue,
            "ProtectHome=tmpfs" => "ProtectHome=yes".to_string(),
            _ => p,
        };
        s.push_str(&p);
        s.push('\n');
    }
    s
}

/// The system unit that applies the switches at boot (ADR 0056 D8): after the firewall unit so
/// the OS firewall's `inet wolos` table is loaded first, before the network comes up so no packet
/// is sent in between. `reload` re-applies the rules (see [`nftables_dropin`]).
pub fn rules_unit_file(wol_tor: &Path) -> String {
    format!(
        "# Generated by wol-tor (ADR 0056).\n[Unit]\nDescription=Tor scopes of this computer\n\
         After=nftables.service\nWants=nftables.service\nBefore=network-pre.target network-online.target\n\
         ConditionPathExists=/etc/wol-tor/switches\n\n[Service]\nType=oneshot\nRemainAfterExit=yes\n\
         ExecStart={w} apply\nExecReload={w} apply --reload-rules\n\n[Install]\nWantedBy=multi-user.target\n",
        w = wol_tor.display()
    )
}

/// Drop-in for `nftables.service`: when the firewall is (re)loaded, the Tor rules are loaded again
/// after it. The firewall file no longer flushes the whole ruleset, so this is a second guard (a
/// firewall restart, or an administrator's own `nft flush ruleset`, would otherwise leave Tor
/// scopes without their rules until the next boot). Named to sort after the OS drop-in.
pub fn nftables_dropin(wol_tor: &Path) -> String {
    format!(
        "# Generated by wol-tor (ADR 0056). Sorts after wolos.conf, which resets ExecReload.\n[Service]\n\
         ExecStartPost=-{w} apply --reload-rules\nExecReload=-{w} apply --reload-rules\n",
        w = wol_tor.display()
    )
}

/// File name of [`nftables_dropin`] (after `wolos.conf` in sort order).
pub const NFTABLES_DROPIN_NAME: &str = "wolos.tor.conf";

/// The user units of the cache refresh (ADR 0053 D4), as file text: (service, timer). The timer
/// is installed and enabled only by `wol-tor keep-ready on` ([`crate::ready`]); default off.
pub fn cache_unit_files(wol_tor: &Path) -> (String, String) {
    let mut service = String::from(
        "# Generated by wol-tor (ADR 0053 D4): keeps Tor's directory documents and guard warm.\n\
         [Unit]\nDescription=Keep Tor ready for Atlas Tor tabs\n\n[Service]\nType=oneshot\n",
    );
    service.push_str(&format!("ExecStart={} cache-run\n", wol_tor.display()));
    service.push_str(concat!(
        "Nice=19\nIOSchedulingClass=idle\n",
        "RuntimeDirectory=wol-tor/cache-run\nRuntimeDirectoryMode=0700\n",
        "StateDirectory=wol-tor/cache\nStateDirectoryMode=0700\nTimeoutStartSec=180\n",
        "Environment=WOL_TOR_KEEP_READY=on\n"
    ));
    for p in tor_sandbox() {
        service.push_str(&p);
        service.push('\n');
    }
    let timer = String::from(
        "# Generated by wol-tor (ADR 0053 D4).\n[Unit]\nDescription=Keep Tor ready for Atlas Tor tabs\n\n\
         [Timer]\nOnStartupSec=10min\nOnUnitActiveSec=6h\nRandomizedDelaySec=30min\n\n\
         [Install]\nWantedBy=timers.target\n",
    );
    (service, timer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g() -> Group {
        Group::new("default").unwrap()
    }

    fn paths() -> Paths {
        Paths::new(Path::new("/run/user/1000"), &g()).unwrap()
    }

    #[test]
    fn browser_unit_has_no_network_and_keeps_nothing() {
        let p = host_properties(&g(), &paths());
        for want in [
            "PrivateNetwork=yes",
            "PrivateUsers=yes",
            "PrivateTmp=yes",
            "ProtectHome=tmpfs",
            "MemorySwapMax=0",
            "LimitCORE=0",
            "Environment=TZ=UTC",
        ] {
            assert!(p.iter().any(|x| x == want), "{want}");
        }
        assert!(p.contains(
            &"BindReadOnlyPaths=/run/user/1000/wol-tor-profile/default/policies:/etc/atlas/policies"
                .to_string()
        ));
        assert!(
            p.iter()
                .filter(|x| x.starts_with("BindReadOnlyPaths="))
                .all(|x| !x.contains("=-")),
            "a missing policy must fail the unit, not be skipped"
        );
        assert!(
            p.contains(
                &"Environment=FONTCONFIG_FILE=/run/user/1000/wol-tor-profile/default/fonts.conf"
                    .to_string()
            )
        );
        assert!(
            p.contains(&"RuntimeDirectory=wol-tor-browser/default".to_string()),
            "the profile and the command socket live on the unit's own tmpfs"
        );
    }

    #[test]
    fn host_argv_wants_tor_and_passes_session_env() {
        let v = host_unit(
            &g(),
            &paths(),
            Path::new("/usr/lib/wol-tor"),
            Path::new("/usr/bin/atlas"),
            &["WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"],
            None,
        );
        let dd = v.iter().position(|x| x == "--").unwrap();
        assert!(v[..dd].contains(&"--unit=atlas-tor-default.service".to_string()));
        assert!(v[..dd].contains(&"--property=Wants=wol-tor-default.service".to_string()));
        assert!(v[..dd].contains(&"--setenv=WAYLAND_DISPLAY".to_string()));
        assert_eq!(
            &v[dd + 1..],
            [
                "/usr/lib/wol-tor",
                "host",
                "--group",
                "default",
                "--",
                "/usr/bin/atlas"
            ]
        );
    }

    #[test]
    fn tor_unit_is_sandboxed_with_a_tmpfs_runtime_dir() {
        let v = tor_unit(&g(), Path::new("/usr/lib/wol-tor"));
        assert_eq!(&v[..4], ["systemd-run", "--user", "--collect", "--quiet"]);
        for want in [
            "--property=RuntimeDirectory=wol-tor/default",
            "--property=RuntimeDirectoryMode=0700",
            "--property=RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
            "--property=MemoryDenyWriteExecute=yes",
            "--property=StandardOutput=null",
            "--property=CapabilityBoundingSet=",
        ] {
            assert!(v.iter().any(|x| x == want), "{want}");
        }
        assert!(
            !v.iter().any(|x| x.contains("PrivateNetwork")),
            "tor needs the network"
        );
        assert_eq!(v.last().map(String::as_str), Some("default"));
    }

    #[test]
    fn with_the_machine_scope_on_the_group_uses_the_machines_tor_and_owns_nothing() {
        let own = host_unit(&g(), &paths(), Path::new("/w"), Path::new("/a"), &[], None);
        let sock = Path::new("/run/wol-tor/gateway/socks");
        let shared = host_unit(
            &g(),
            &paths(),
            Path::new("/w"),
            Path::new("/a"),
            &[],
            Some(sock),
        );
        assert!(own.contains(&"--property=Wants=wol-tor-default.service".to_string()));
        assert!(
            !shared.iter().any(|x| x.contains("wol-tor-default.service")),
            "no second tor"
        );
        let host = shared.iter().position(|x| x == "host").unwrap();
        assert_eq!(
            &shared[host..host + 5],
            [
                "host",
                "--group",
                "default",
                "--tor-socks",
                "/run/wol-tor/gateway/socks"
            ]
        );
    }

    #[test]
    fn the_rules_unit_runs_after_the_firewall_and_before_the_network() {
        let u = rules_unit_file(Path::new("/usr/lib/wol-tor"));
        assert!(u.contains("After=nftables.service\n"));
        assert!(u.contains("Before=network-pre.target network-online.target\n"));
        assert!(
            u.contains("ExecStart=/usr/lib/wol-tor apply\n")
                && u.contains("ExecReload=/usr/lib/wol-tor apply --reload-rules\n")
        );
        let d = nftables_dropin(Path::new("/usr/lib/wol-tor"));
        assert!(
            d.contains("ExecStartPost=-/usr/lib/wol-tor apply --reload-rules")
                && d.contains("ExecReload=-")
        );
        assert!(
            !d.contains("ExecReload=\n"),
            "must not reset the OS firewall's own ExecReload"
        );
        // Drop-ins apply in file-name order; the OS one resets ExecReload, so ours must come later.
        assert!(NFTABLES_DROPIN_NAME > "wolos.conf");
    }

    #[test]
    fn the_gateway_keeps_tors_state_across_restarts() {
        let u = gateway_unit_file(Path::new("/usr/lib/wol-tor"), "tor");
        assert!(
            u.contains("StateDirectory=wol-tor/gateway\n")
                && u.contains("StateDirectoryMode=0700\n")
        );
        assert!(u.contains("RuntimeDirectory=wol-tor/gateway\n"));
    }

    #[test]
    fn the_gateway_unit_runs_tor_as_its_own_user_in_the_sandbox() {
        let u = gateway_unit_file(Path::new("/usr/lib/wol-tor"), "tor");
        assert!(
            u.contains("User=tor\n")
                && u.contains("ExecStart=/usr/lib/wol-tor tor-run --gateway\n")
        );
        for want in [
            "NoNewPrivileges=yes",
            "MemoryDenyWriteExecute=yes",
            "CapabilityBoundingSet=\n",
            "ProtectHome=yes",
            "LimitCORE=0",
            "RuntimeDirectory=wol-tor/gateway",
        ] {
            assert!(u.contains(want), "{want}");
        }
        assert!(!u.contains("PrivateUsers") && !u.contains("PrivateNetwork"));
    }

    #[test]
    fn cache_units_are_idle_priority_and_opt_in() {
        let (s, t) = cache_unit_files(Path::new("/usr/lib/wol-tor"));
        assert!(s.contains("ExecStart=/usr/lib/wol-tor cache-run\n"));
        assert!(s.contains("Nice=19\n") && s.contains("IOSchedulingClass=idle\n"));
        assert!(s.contains("Environment=WOL_TOR_KEEP_READY=on\n"));
        assert!(t.contains("OnUnitActiveSec=6h\n"));
        assert!(
            t.contains("[Install]\nWantedBy=timers.target\n"),
            "enabled by wol-tor on first use, not by a preset"
        );
    }
}
