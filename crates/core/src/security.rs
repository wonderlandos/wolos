// SPDX-License-Identifier: BSD-2-Clause
//! Settings > Privacy & Security backend: the firewall, the kernel hardening WolOS ships and what
//! each app permission really enforces (docs/modules/security.md).

/// Kernel hardening shipped in [`HARDENING_FILE`] (`files/usr/lib/sysctl.d`, test-checked):
/// (sysctl, value, why). ASLR is already the Linux default (`kernel.randomize_va_space=2`).
pub const HARDENING: &[(&str, &str, &str)] = &[
    (
        "kernel.kptr_restrict",
        "2",
        "kernel addresses stay hidden from every account (exploits need them)",
    ),
    (
        "kernel.dmesg_restrict",
        "1",
        "the kernel log (addresses, devices, USB serials) is for administrators",
    ),
    (
        "kernel.yama.ptrace_scope",
        "1",
        "apps can debug only their own children, not read other running apps' memory",
    ),
    (
        "kernel.unprivileged_bpf_disabled",
        "1",
        "only administrators load BPF programs into the kernel",
    ),
    (
        "fs.protected_hardlinks",
        "1",
        "no hard links to other accounts' files (link tricks in shared folders)",
    ),
    (
        "fs.protected_symlinks",
        "1",
        "no following other accounts' symlinks in shared folders like /tmp",
    ),
    (
        "net.ipv4.tcp_syncookies",
        "1",
        "a flood of half-open connections cannot exhaust the network stack",
    ),
];

/// Where [`HARDENING`] is set at boot (systemd-sysctl): the shipped default, owned by the
/// package (`files/usr/lib/sysctl.d`, image-test-checked). A local change goes in a drop-in whose
/// name sorts after `90-wolos.conf` (systemd-sysctl applies files by name, the last value wins);
/// a file of the same name in `/etc/sysctl.d` ([`HARDENING_MASK`]) would replace this one whole,
/// dropping every line of the hardening. Settings and
/// `wol security` read the running values from `/proc/sys`, so either way they show what the
/// kernel really has.
pub const HARDENING_FILE: &str = "/usr/lib/sysctl.d/90-wolos.conf";

/// Same-name file in `/etc/sysctl.d` that would mask [`HARDENING_FILE`] (sysctl.d precedence).
pub const HARDENING_MASK: &str = "/etc/sysctl.d/90-wolos.conf";

/// Syslog daemons that could send the log over the network (WolOS ships journald only).
pub const NETWORK_LOGGERS: &[&str] = &["rsyslog.service", "syslog-ng@default.service"];

/// One hardening setting as the kernel has it now.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Check {
    pub key: String,
    pub want: String,
    /// None: unknown (the sysctl is missing or unreadable).
    pub now: Option<String>,
}

impl Check {
    pub fn ok(&self) -> bool {
        self.now.as_deref() == Some(self.want.as_str())
    }
}

/// Probe: fixed argv -> stdout, None when it failed (wol-admind's and the CLI's runners).
pub type Probe<'a> = &'a dyn Fn(&[&str]) -> Option<String>;

/// `/proc/sys` path of a sysctl key.
pub fn proc_sys(key: &str) -> String {
    format!("/proc/sys/{}", key.replace('.', "/"))
}

/// [`HARDENING`] against the running kernel (`/proc/sys`).
pub fn hardening(probe: Probe) -> Vec<Check> {
    HARDENING
        .iter()
        .map(|(k, v, _)| Check {
            key: (*k).into(),
            want: (*v).into(),
            now: probe(&["cat", &proc_sys(k)])
                .map(|o| o.trim().to_owned())
                .filter(|o| !o.is_empty()),
        })
        .collect()
}

/// What Privacy & Security shows.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// Firewall on at boot (nftables.service enabled); None: unknown.
    pub firewall: Option<bool>,
    /// nftables loaded now (`systemctl is-active`); None: unknown.
    pub firewall_running: Option<bool>,
    /// Remote Login (sshd) on at boot.
    pub remote_login: Option<bool>,
    /// No syslog daemon that could send the log over the network ([`NETWORK_LOGGERS`]).
    pub system_log_private: bool,
    pub hardening: Vec<Check>,
}

/// `systemctl is-active` output -> running.
pub fn parse_active(text: &str) -> Option<bool> {
    match text.trim() {
        "active" | "reloading" => Some(true),
        "inactive" | "failed" | "deactivating" => Some(false),
        _ => None,
    }
}

pub fn status(probe: Probe) -> Status {
    let unit = |u: &str| {
        probe(&["systemctl", "show", "-p", "UnitFileState", "--value", u])
            .and_then(|o| crate::sharing::parse_enabled(&o))
    };
    Status {
        firewall: unit("nftables.service"),
        firewall_running: probe(&[
            "systemctl",
            "show",
            "-p",
            "ActiveState",
            "--value",
            "nftables.service",
        ])
        .and_then(|o| parse_active(&o)),
        remote_login: unit("sshd.service"),
        system_log_private: NETWORK_LOGGERS.iter().all(|u| unit(u) == Some(false)),
        hardening: hardening(probe),
    }
}

/// How a privacy permission is enforced. Settings shows the honest wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Enforcement {
    /// The compositor refuses it (compositor permissions, [`HYPR_PERMISSIONS`]).
    Compositor,
    /// Recorded and shown to helpers that ask; nothing stops an app that ignores it yet.
    Advisory,
    /// wold refuses the call (libwol services, ADR 0020).
    System,
}

/// Privacy permissions Settings explains, with how each is enforced and why (one line): every
/// grant in [`crate::consent::CAPABILITIES`], plus what is never a grant there:
/// [`crate::consent::SCREEN_CAPTURE`] (the compositor and the portal decide, [`HYPR_PERMISSIONS`])
/// and the devices and places an app may reach (camera, microphone, location, files).
pub const PERMISSIONS: &[(&str, Enforcement, &str)] = &[
    (
        "screen-capture",
        Enforcement::Compositor,
        "only the system's screenshot and recording tools copy the screen directly; apps go through the portal, which asks which screen to share",
    ),
    (
        "notifications",
        Enforcement::System,
        "apps post notifications through WolOS, which checks your choice before showing one",
    ),
    (
        "input-injection",
        Enforcement::Advisory,
        "virtual keyboards are not tied to an app by the compositor",
    ),
    (
        "clipboard-history",
        Enforcement::Advisory,
        "Wayland gives the clipboard to the focused app; the history is for helpers that ask",
    ),
    (
        "window-list",
        Enforcement::Advisory,
        "window titles go to helpers that ask through WolOS",
    ),
    (
        "packages-install",
        Enforcement::System,
        "only WolOS's system service installs system apps, and it checks your choice first",
    ),
    (
        "packages-remove",
        Enforcement::System,
        "only WolOS's system service removes system apps, and it checks your choice first",
    ),
    (
        "packages-update",
        Enforcement::System,
        "only WolOS's system service updates the system, and only to a release WolOS signed",
    ),
    (
        "packages-unreviewed",
        Enforcement::System,
        "apps built on this computer are installed only by WolOS's system service, after your choice",
    ),
    (
        "camera",
        Enforcement::Advisory,
        "/dev/video* belong to group video, which every account needs for graphics too",
    ),
    (
        "microphone",
        Enforcement::Advisory,
        "PipeWire gives the microphone to every app of the account; there is no per-app gate yet",
    ),
    (
        "location",
        Enforcement::Advisory,
        "WolOS ships no location service; nothing reports a position",
    ),
    (
        "files-outside-home",
        Enforcement::Advisory,
        "Unix permissions apply; apps run as the account, without a per-app sandbox",
    ),
    (
        "clipboard-read",
        Enforcement::System,
        "apps ask WolOS for the text you copied, which checks your choice before handing it over",
    ),
    (
        "clipboard-write",
        Enforcement::System,
        "apps ask WolOS to change the clipboard, which checks your choice first",
    ),
    (
        "app-preferences",
        Enforcement::System,
        "WolOS keeps each app's preferences apart and checks your choice before reading or changing them",
    ),
    (
        "file-panels",
        Enforcement::System,
        "apps ask WolOS to show a panel, and only the file you pick reaches the app",
    ),
    (
        "power-info",
        Enforcement::System,
        "apps ask WolOS for the battery level, which checks your choice first",
    ),
    (
        "keep-awake",
        Enforcement::System,
        "apps ask WolOS to hold the machine awake, which checks your choice first and lets go when the app ends",
    ),
    (
        "network-info",
        Enforcement::System,
        "apps ask WolOS whether the computer is online, which checks your choice first and never says where you are",
    ),
    (
        "audio-info",
        Enforcement::System,
        "apps ask WolOS for the volume and what is playing, which checks your choice first",
    ),
    (
        "audio-control",
        Enforcement::System,
        "apps ask WolOS to change the volume, which checks your choice first",
    ),
    (
        "search",
        Enforcement::System,
        "apps ask WolOS to search, which checks your choice and hands back only titles",
    ),
    (
        "search-provider",
        Enforcement::System,
        "only apps you allow are asked for search rows, and each sees only the text of the search",
    ),
    (
        "account-info",
        Enforcement::System,
        "apps ask WolOS, which answers only about the account running them",
    ),
    (
        "ipc",
        Enforcement::System,
        "WolOS routes each message, checks that you allowed that pair of apps and tells the receiver who sent it",
    ),
    (
        "devices",
        Enforcement::System,
        "apps ask WolOS, which lists device names without serial numbers or addresses",
    ),
    (
        "share",
        Enforcement::System,
        "WolOS shows the person what would be shared, and nothing leaves until they pick where it goes",
    ),
    (
        "background",
        Enforcement::System,
        "WolOS runs only the app's own program, as your account, until the app or the computer stops it",
    ),
];

/// [`PERMISSIONS`] entry of a capability.
pub fn enforcement(capability: &str) -> Option<Enforcement> {
    PERMISSIONS.iter().find(|p| p.0 == capability).map(|p| p.1)
}

/// Programs that may copy the screen (and see the pointer in it) directly: the portal (it asks
/// which screen to share), the shell and its screenshot, recording and Overview tools.
pub const SCREENCOPY_SYSTEM: &[&str] = &[
    "/usr/lib/xdg-desktop-portal-hyprland",
    "/usr/bin/grim",
    "/usr/bin/wf-recorder",
    "/usr/bin/wol-window-shot",
    "/usr/bin/wol-shell",
];

/// Compositor plugins load only from the root-owned plugin folder (hyprbars, wol-hypr).
pub const PLUGIN_DIR: &str = "/usr/lib/hyprland/plugins";

/// Shipped compositor permission rules (`hypr/permissions.lua`, test-checked).
pub const HYPR_PERMISSIONS: &str = "/usr/share/wolos/hypr/permissions.lua";

/// Compositor `hl.permission` rules: only [`SCREENCOPY_SYSTEM`] may copy the screen; nobody may
/// capture input; plugins only from [`PLUGIN_DIR`]. First match wins, so each deny comes last;
/// keyboards stay allowed (the compositor's default). Nothing is left to "ask" (no dialog ships).
/// The file is static: apps record the screen through the portal, which asks which screen to
/// share, so no consent grant adds a program here (docs/modules/security.md, App permissions).
pub fn hypr_permissions() -> String {
    let mut s = String::from(
        "-- WolOS privacy (docs/modules/security.md): who may copy the screen, capture input\n\
         -- or load compositor plugins. Generated by wol_core::security::hypr_permissions.\n\
         hl.config({\n    ecosystem = {\n        enforce_permissions = true,\n        no_update_news = true,\n        no_donation_nag = true,\n    },\n})\n",
    );
    let rule = |re: &str, kind: &str, mode: &str| {
        // A long bracket keeps a regex with `\` readable (no escapes inside it).
        let bin = if re.contains('\\') && !re.contains("]]") {
            format!("[[{re}]]")
        } else {
            crate::theme::hypr::lua_str(re)
        };
        format!("hl.permission({{ binary = {bin}, type = \"{kind}\", mode = \"{mode}\" }})\n")
    };
    for a in SCREENCOPY_SYSTEM {
        let re = format!("^{}$", regex_quote(a));
        s += &rule(&re, "screencopy", "allow");
        s += &rule(&re, "cursorpos", "allow");
    }
    s += &rule(".*", "screencopy", "deny");
    s += &rule(".*", "cursorpos", "deny");
    s += &rule(".*", "input-capture", "deny");
    s += &rule(
        &format!("^{}/[^/]+\\.so$", regex_quote(PLUGIN_DIR)),
        "plugin",
        "allow",
    );
    s += &rule(".*", "plugin", "deny");
    s
}

/// Path as a literal regex (the compositor matches permission rules with RE2).
fn regex_quote(p: &str) -> String {
    let mut s = String::new();
    for c in p.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            s.push('\\');
        }
        s.push(c);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardening_table_is_justified_and_unique() {
        let mut keys: Vec<&str> = HARDENING.iter().map(|h| h.0).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), HARDENING.len());
        for (k, v, why) in HARDENING {
            assert!(
                ["kernel.", "fs.", "net."].iter().any(|p| k.starts_with(p)),
                "{k}"
            );
            assert!(v.parse::<i64>().is_ok(), "{k}={v}");
            assert!(!why.is_empty() && !why.contains('\n'), "{k}: one line");
        }
        assert!(
            !keys.contains(&"kernel.yama.ptrace_scope") || {
                let v = HARDENING.iter().find(|h| h.0 == "kernel.yama.ptrace_scope");
                v.is_some_and(|h| h.1 == "1")
            },
            "debugging own children stays (developers)"
        );
        assert_eq!(
            proc_sys("kernel.yama.ptrace_scope"),
            "/proc/sys/kernel/yama/ptrace_scope"
        );
    }

    fn probe(
        answers: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&[&str]) -> Option<String> {
        move |argv| {
            let line = argv.join(" ");
            answers
                .iter()
                .find(|a| a.0 == line)
                .map(|a| a.1.to_string())
        }
    }

    #[test]
    fn status_reports_the_running_system() {
        const MACHINE: &[(&str, &str)] = &[
            ("cat /proc/sys/kernel/kptr_restrict", "2\n"),
            ("cat /proc/sys/kernel/dmesg_restrict", "0\n"),
            (
                "systemctl show -p UnitFileState --value nftables.service",
                "enabled\n",
            ),
            (
                "systemctl show -p ActiveState --value nftables.service",
                "active\n",
            ),
            (
                "systemctl show -p UnitFileState --value sshd.service",
                "disabled\n",
            ),
            (
                "systemctl show -p UnitFileState --value rsyslog.service",
                "\n",
            ),
            (
                "systemctl show -p UnitFileState --value syslog-ng@default.service",
                "\n",
            ),
        ];
        let st = status(&probe(MACHINE));
        assert_eq!(
            (
                st.firewall,
                st.firewall_running,
                st.remote_login,
                st.system_log_private
            ),
            (Some(true), Some(true), Some(false), true)
        );
        let get = |k: &str| st.hardening.iter().find(|c| c.key == k).unwrap();
        assert!(get("kernel.kptr_restrict").ok());
        assert!(
            !get("kernel.dmesg_restrict").ok(),
            "set at boot by sysctl.d"
        );
        assert_eq!(get("fs.protected_symlinks").now, None);
        let user = status(&probe(&[]));
        assert_eq!((user.firewall, user.firewall_running), (None, None));
        assert!(!user.system_log_private);
        assert_eq!(parse_active("failed\n"), Some(false));
        assert_eq!(parse_active("Permission denied"), None);
    }

    #[test]
    fn every_permission_says_how_it_is_enforced() {
        for c in crate::consent::CAPABILITIES {
            assert!(enforcement(c).is_some(), "{c}");
        }
        for c in ["camera", "microphone", "location", "files-outside-home"] {
            assert_eq!(enforcement(c), Some(Enforcement::Advisory), "{c}");
        }
        assert_eq!(
            enforcement(crate::consent::SCREEN_CAPTURE),
            Some(Enforcement::Compositor)
        );
        // Explained here, never granted in the consent file.
        assert!(!crate::consent::CAPABILITIES.contains(&crate::consent::SCREEN_CAPTURE));
        assert!(!crate::consent::is_valid_grant(
            "/usr/bin/x",
            crate::consent::SCREEN_CAPTURE
        ));
        assert_eq!(enforcement("notifications"), Some(Enforcement::System));
        for g in shared::packages::GRANTS {
            assert_eq!(enforcement(g), Some(Enforcement::System), "{g}");
        }
        assert!(crate::consent::is_valid_grant(
            "/usr/bin/x",
            "notifications"
        ));
        assert_eq!(enforcement("teleport"), None);
        for (c, _, why) in PERMISSIONS {
            assert!(!why.is_empty() && !why.contains('\n'), "{c}");
        }
    }

    #[test]
    fn compositor_rules_allow_system_tools_only() {
        let s = hypr_permissions();
        let rules: Vec<&str> = s
            .lines()
            .filter(|l| l.starts_with("hl.permission("))
            .collect();
        let allowed: Vec<&str> = rules
            .iter()
            .filter_map(|r| {
                r.strip_prefix("hl.permission({ binary = \"")?
                    .strip_suffix("\", type = \"screencopy\", mode = \"allow\" })")
            })
            .collect();
        assert_eq!(
            allowed,
            [
                "^/usr/lib/xdg-desktop-portal-hyprland$",
                "^/usr/bin/grim$",
                "^/usr/bin/wf-recorder$",
                "^/usr/bin/wol-window-shot$",
                "^/usr/bin/wol-shell$",
            ]
        );
        let tail = &rules[rules.len() - 5..];
        assert_eq!(
            tail,
            [
                "hl.permission({ binary = \".*\", type = \"screencopy\", mode = \"deny\" })",
                "hl.permission({ binary = \".*\", type = \"cursorpos\", mode = \"deny\" })",
                "hl.permission({ binary = \".*\", type = \"input-capture\", mode = \"deny\" })",
                "hl.permission({ binary = [[^/usr/lib/hyprland/plugins/[^/]+\\.so$]], type = \"plugin\", mode = \"allow\" })",
                "hl.permission({ binary = \".*\", type = \"plugin\", mode = \"deny\" })",
            ]
        );
        assert!(
            !s.contains("\"ask\""),
            "no dialog ships: nothing may wait for one"
        );
        assert!(s.contains("enforce_permissions = true,"));
        // hyprland-update-screen crashed at every first login (no news window in WolOS).
        assert!(s.contains("no_update_news = true,"));
        assert!(s.contains("no_donation_nag = true,"));
    }

    #[test]
    fn paths_become_literal_regexes() {
        assert_eq!(regex_quote("/opt/x(1)/a.b"), "/opt/x\\(1\\)/a\\.b");
        assert_eq!(regex_quote("/usr/bin/grim"), "/usr/bin/grim");
    }
}
