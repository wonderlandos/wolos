//! Settings > Privacy & Security backend: the firewall, the kernel hardening WolOS ships and what
//! each app permission really enforces on FreeBSD (archicture/modules/security.md).

/// Kernel hardening shipped in `/etc/sysctl.conf.local` (desktop/overlay, test-checked):
/// (sysctl, value, why). ASLR is already the FreeBSD 15 default (`kern.elf64.aslr.enable=1`).
pub const HARDENING: &[(&str, &str, &str)] = &[
    (
        "security.bsd.see_other_uids",
        "0",
        "accounts cannot list each other's processes (command lines can hold secrets)",
    ),
    (
        "security.bsd.see_other_gids",
        "0",
        "nor the processes of groups they are not in",
    ),
    (
        "security.bsd.unprivileged_read_msgbuf",
        "0",
        "the kernel log (addresses, devices, USB serials) is for administrators",
    ),
    (
        "security.bsd.hardlink_check_uid",
        "1",
        "no hard links to other accounts' files (link tricks in shared folders)",
    ),
    (
        "security.bsd.hardlink_check_gid",
        "1",
        "nor to files of groups the account is not in",
    ),
    (
        "kern.randompid",
        "1",
        "process IDs cannot be guessed (PID races against helpers)",
    ),
];

/// rc.conf settings WolOS ships for security (image/rcconf.sh and image/installerconfig, test-checked).
pub const RC_DEFAULTS: &[(&str, &str, &str)] = &[
    ("pf_enable", "YES", "the firewall is on from the first boot"),
    (
        "pf_rules",
        crate::sharing::PF_RULES,
        "the WolOS ruleset: nothing comes in unasked",
    ),
    (
        "syslogd_flags",
        "-ss",
        "the system log listens on no network socket",
    ),
    ("sendmail_enable", "NONE", "no mail daemon"),
    ("clear_tmp_enable", "YES", "/tmp starts empty at every boot"),
];

/// One hardening setting as the kernel has it now.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Check {
    pub key: String,
    pub want: String,
    /// None: unknown (not FreeBSD, or the sysctl is missing).
    pub now: Option<String>,
}

impl Check {
    pub fn ok(&self) -> bool {
        self.now.as_deref() == Some(self.want.as_str())
    }
}

/// Probe: fixed argv -> stdout, None when it failed (wol-admind's and the CLI's runners).
pub type Probe<'a> = &'a dyn Fn(&[&str]) -> Option<String>;

/// [`HARDENING`] against the running kernel (`sysctl -n`).
pub fn hardening(probe: Probe) -> Vec<Check> {
    HARDENING
        .iter()
        .map(|(k, v, _)| Check {
            key: (*k).into(),
            want: (*v).into(),
            now: probe(&["sysctl", "-n", k])
                .map(|o| o.trim().to_owned())
                .filter(|o| !o.is_empty()),
        })
        .collect()
}

/// What Privacy & Security shows.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// Firewall on at boot (`pf_enable`); None: unknown.
    pub firewall: Option<bool>,
    /// pf filtering now (`pfctl -s info`, administrators only); None: not known to this account.
    pub firewall_running: Option<bool>,
    /// Remote Login (sshd) on at boot.
    pub remote_login: Option<bool>,
    /// The system log listens on no network socket (`syslogd -ss`).
    pub system_log_private: bool,
    pub hardening: Vec<Check>,
}

/// `pfctl -s info` -> filtering on (`Status: Enabled ...`).
pub fn parse_pf_info(text: &str) -> Option<bool> {
    let l = text.lines().find(|l| l.starts_with("Status: "))?;
    Some(l["Status: ".len()..].starts_with("Enabled"))
}

pub fn status(probe: Probe) -> Status {
    let rc =
        |var: &str| probe(&["sysrc", "-n", var]).and_then(|o| crate::sharing::parse_rc_yes(&o));
    Status {
        firewall: rc("pf_enable"),
        firewall_running: probe(&["pfctl", "-s", "info"]).and_then(|o| parse_pf_info(&o)),
        remote_login: rc("sshd_enable"),
        system_log_private: probe(&["sysrc", "-n", "syslogd_flags"])
            .is_some_and(|f| f.split_whitespace().any(|w| w == "-ss")),
        hardening: hardening(probe),
    }
}

/// How a privacy permission is enforced on FreeBSD. Settings shows the honest wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Enforcement {
    /// The compositor refuses it (Hyprland permissions, [`HYPR_PERMISSIONS`]).
    Compositor,
    /// Recorded and shown to helpers that ask; FreeBSD cannot stop an app that ignores it.
    Advisory,
}

/// Privacy permissions: [`crate::consent::CAPABILITIES`] plus the devices and places an app may
/// reach, with how each is enforced and why (one line).
pub const PERMISSIONS: &[(&str, Enforcement, &str)] = &[
    (
        "screen-capture",
        Enforcement::Compositor,
        "only the system's screenshot and recording tools copy the screen directly; apps go through the portal, which asks which screen to share",
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
        "camera",
        Enforcement::Advisory,
        "/dev/video* belong to group video, which every account needs for graphics too",
    ),
    (
        "microphone",
        Enforcement::Advisory,
        "/dev/dsp plays and records on the same device, so recording cannot be split off",
    ),
    (
        "location",
        Enforcement::Advisory,
        "FreeBSD has no location service; nothing reports a position",
    ),
    (
        "files-outside-home",
        Enforcement::Advisory,
        "Unix permissions apply; apps run as the account, without a per-app sandbox",
    ),
];

/// [`PERMISSIONS`] entry of a capability.
pub fn enforcement(capability: &str) -> Option<Enforcement> {
    PERMISSIONS.iter().find(|p| p.0 == capability).map(|p| p.1)
}

/// Programs that may copy the screen (and see the pointer in it) directly: the portal (it asks
/// which screen to share), the shell and its screenshot, recording and Mission Control tools.
pub const SCREENCOPY_SYSTEM: &[&str] = &[
    "/usr/local/libexec/xdg-desktop-portal-hyprland",
    "/usr/local/bin/grim",
    "/usr/local/bin/wf-recorder",
    "/usr/local/bin/wol-window-shot",
    "/usr/local/bin/wol-shell",
];

/// Compositor plugins load only from the root-owned plugin folder (hyprbars, wol-hypr).
pub const PLUGIN_DIR: &str = "/usr/local/lib/hyprland/plugins";

/// Shipped Hyprland permission rules (`hypr/permissions.conf`, test-checked).
pub const HYPR_PERMISSIONS: &str = "/usr/local/share/wolos/hypr/permissions.conf";

/// Hyprland `permission` rules: system tools and `granted` apps may copy the screen; nobody may
/// capture input; plugins only from [`PLUGIN_DIR`]. First match wins, so each deny comes last;
/// keyboards stay allowed (Hyprland's default). Nothing is left to "ask" (no dialog ships).
pub fn hypr_permissions(granted: &[&str]) -> String {
    let mut s = String::from(
        "# WolOS privacy (archicture/modules/security.md): who may copy the screen, capture input\n\
         # or load compositor plugins. Generated by wol_core::security::hypr_permissions.\n\
         ecosystem {\n    enforce_permissions = true\n}\n",
    );
    let apps = SCREENCOPY_SYSTEM.iter().copied().chain(
        granted
            .iter()
            .copied()
            .filter(|a| crate::consent::is_valid_grant(a, "screen-capture")),
    );
    for a in apps {
        let re = regex_quote(a);
        s.push_str(&format!("permission = ^{re}$, screencopy, allow\n"));
        s.push_str(&format!("permission = ^{re}$, cursorpos, allow\n"));
    }
    s.push_str("permission = .*, screencopy, deny\npermission = .*, cursorpos, deny\n");
    s.push_str("permission = .*, input-capture, deny\n");
    s.push_str(&format!(
        "permission = ^{}/[^/]+\\.so$, plugin, allow\npermission = .*, plugin, deny\n",
        regex_quote(PLUGIN_DIR)
    ));
    s
}

/// Path as a literal regex (Hyprland matches permission rules with RE2).
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
                k.starts_with("security.bsd.") || k.starts_with("kern."),
                "{k}"
            );
            assert!(v.parse::<i64>().is_ok(), "{k}={v}");
            assert!(!why.is_empty() && !why.contains('\n'), "{k}: one line");
        }
        assert!(
            !keys.contains(&"security.bsd.unprivileged_proc_debug"),
            "own-process debugging stays (developers); other accounts' never was allowed"
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
            ("sysctl -n security.bsd.see_other_uids", "0\n"),
            ("sysctl -n kern.randompid", "0\n"),
            ("sysrc -n pf_enable", "YES\n"),
            ("sysrc -n sshd_enable", "NO\n"),
            ("sysrc -n syslogd_flags", "-ss\n"),
            (
                "pfctl -s info",
                "Status: Enabled for 0 days 00:12:01           Debug: Urgent\n\nState Table\n",
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
        assert!(get("security.bsd.see_other_uids").ok());
        assert!(
            !get("kern.randompid").ok(),
            "set at boot by sysctl.conf.local"
        );
        assert_eq!(get("security.bsd.see_other_gids").now, None);
        let user = status(&probe(&[]));
        assert_eq!((user.firewall, user.firewall_running), (None, None));
        assert!(!user.system_log_private);
        assert_eq!(parse_pf_info("Status: Disabled for 0 days\n"), Some(false));
        assert_eq!(parse_pf_info("pfctl: /dev/pf: Permission denied\n"), None);
    }

    #[test]
    fn every_permission_says_how_it_is_enforced() {
        for c in crate::consent::CAPABILITIES {
            assert!(enforcement(c).is_some(), "{c}");
        }
        for c in ["camera", "microphone", "location", "files-outside-home"] {
            assert_eq!(enforcement(c), Some(Enforcement::Advisory), "{c}");
        }
        assert_eq!(enforcement("screen-capture"), Some(Enforcement::Compositor));
        assert_eq!(enforcement("teleport"), None);
        for (c, _, why) in PERMISSIONS {
            assert!(!why.is_empty() && !why.contains('\n'), "{c}");
        }
    }

    #[test]
    fn compositor_rules_allow_system_tools_and_granted_apps_only() {
        let s = hypr_permissions(&["/usr/local/bin/obs", "relative/app", "/opt/x(1)"]);
        let rules: Vec<&str> = s.lines().filter(|l| l.starts_with("permission")).collect();
        let allowed: Vec<&str> = rules
            .iter()
            .filter(|r| r.ends_with(", screencopy, allow"))
            .map(|r| &r["permission = ".len()..r.len() - ", screencopy, allow".len()])
            .collect();
        assert_eq!(
            allowed,
            [
                "^/usr/local/libexec/xdg-desktop-portal-hyprland$",
                "^/usr/local/bin/grim$",
                "^/usr/local/bin/wf-recorder$",
                "^/usr/local/bin/wol-window-shot$",
                "^/usr/local/bin/wol-shell$",
                "^/usr/local/bin/obs$",
                "^/opt/x\\(1\\)$",
            ]
        );
        let tail = &rules[rules.len() - 5..];
        assert_eq!(
            tail,
            [
                "permission = .*, screencopy, deny",
                "permission = .*, cursorpos, deny",
                "permission = .*, input-capture, deny",
                "permission = ^/usr/local/lib/hyprland/plugins/[^/]+\\.so$, plugin, allow",
                "permission = .*, plugin, deny",
            ]
        );
        assert!(
            !s.contains(", ask"),
            "no dialog ships: nothing may wait for one"
        );
        assert!(s.contains("enforce_permissions = true"));
    }
}
