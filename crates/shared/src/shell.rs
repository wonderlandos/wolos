// SPDX-License-Identifier: BSD-2-Clause
//! Shell socket contract: panel names `wol shell toggle|show|hide <panel>` accepts.
pub const PANELS: &[&str] = &[
    "switcher",
    "notifications",
    "session",
    "clipboard",
    "osd",
    // `show` after the mic-mute key: the OSD with the microphone state (wol-volume micmute).
    "osd-mic",
    "control-center",
    "overview",
    "screenshot",
    "switcher-prev",
    "screen-record",
    // Edit Widgets on the desktop (ADR 0010).
    "widgets",
    // ⌥⌘Esc picker.
    "force-quit",
    // ⇧⌘/: every keyboard shortcut on one sheet.
    "shortcuts",
    // Files dropped on the desktop, kept to drag out again.
    "shelf",
    // `show` moves the keyboard into the bar or the Dock (⌃F2, ⌃F3), `hide` gives it back.
    "menubar",
    "dock",
    // `hide` dismisses every banner on screen (Esc).
    "banners",
    // `hide` closes every transient panel, Search and the widget gallery (Esc).
    "transient",
    // `show` opens the session panel on its Log Out / Restart / Shut Down question (the WolOS menu).
    "session-logout",
    "session-restart",
    "session-shutdown",
    // ⌃⌘T: every theme as a preview card (ADR 0031 themes v2).
    "themes",
];

/// `approve <json>` on wol-shell.sock: a helper (`wol mcp`, a `WOL_HELPER` script) asks before a
/// change. The shell shows the approval sheet and answers on the same connection with one
/// [`Answer`] line. Administrator changes need the password and run in the shell (wol-admind);
/// the helper never sees the password.
pub const APPROVE: &str = "approve";

/// The sheet answers [`Answer::TimedOut`] after this long (counted from arrival, so a queued
/// request waits no longer). Below the 60 s request timeout common MCP clients use.
pub const APPROVAL_TIMEOUT_SECS: u32 = 50;
/// How long a requester waits for the answer line before treating it as no answer.
pub const APPROVAL_WAIT_SECS: u32 = APPROVAL_TIMEOUT_SECS + 10;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Approval {
    /// Who asks (MCP client name), shown on the sheet.
    pub helper: String,
    /// Catalogue verb, for the audit log.
    pub tool: String,
    pub request: ApprovalRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum ApprovalRequest {
    Wifi {
        on: bool,
    },
    WifiJoin {
        ssid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<String>,
    },
    Bluetooth {
        on: bool,
    },
    /// Boot this boot environment next (restore point).
    BootEnvironment {
        name: String,
    },
    /// System packages for every account (wold, ADR 0021): `verb` is `install`, `remove`,
    /// `update` (no names = everything) or `update-holding` (everything except `names`); the shell
    /// runs it through wol-admind after the password.
    Packages {
        verb: String,
        #[serde(default)]
        names: Vec<String>,
    },
    /// A package built on this computer (the community package collection, recipe; wold, ADR 0034): `staged` is wold's own
    /// root-only copy. Unreviewed code whose install steps run as root, so the shell runs it
    /// through wol-admind after the administrator password (audit C7-1). `name` and `version`
    /// are what the copy's `.PKGINFO` says (`pkgname`, `pkgver`), `sha256` the copy's SHA-256
    /// (64 lowercase hex), all read by wold from the staged copy itself. Every field is required:
    /// a sheet without them does not parse, so nothing runs (fail closed).
    PackageFile {
        staged: String,
        name: String,
        version: String,
        sha256: String,
    },
    /// Undo the last system update (wold): through wol-admind after the password.
    SystemRollback,
    /// Switch the system update channel (wold): through wol-admind after the password.
    SystemChannel {
        channel: String,
    },
    /// A Doctor fix (`wol doctor fix <id>`, docs/modules/doctor.md) asked by a helper or a system
    /// web app (ADR 0048): the check's wol-admind requests, run by the shell in order after the
    /// password, as `wol doctor fix` sends them from a terminal. `steps` are `[action, args..]`
    /// (`crate::admin::AdminAction` names); wold relays only [`DOCTOR_ACTIONS`], at most
    /// [`MAX_DOCTOR_STEPS`], and the sheet shows every step, not only the title.
    DoctorFix {
        id: String,
        title: String,
        steps: Vec<Vec<String>>,
    },
    /// A user-level change the helper runs itself once allowed. `what` is the sentence end
    /// ("switch to Dark appearance"); `capability` is what Always Allow grants.
    Change {
        capability: String,
        what: String,
    },
}

/// The wol-admind actions a Doctor fix runs (`controller::doctor`, test-checked there): nothing
/// else rides on a [`ApprovalRequest::DoctorFix`] sheet.
pub const DOCTOR_ACTIONS: &[crate::admin::AdminAction] = &[
    crate::admin::AdminAction::PkgInstall,
    crate::admin::AdminAction::PkgUpgrade,
    crate::admin::AdminAction::Firewall,
    crate::admin::AdminAction::RemoteLogin,
    crate::admin::AdminAction::HwLoad,
];
/// Most wol-admind requests one Doctor fix sends.
pub const MAX_DOCTOR_STEPS: usize = 8;

/// Most bytes of a web app call's parameters, the one cap for the page, the host, `wol webapp-call`
/// and wold (QA-100 H9-19: they had 64 KiB, 60 KiB and a line limit counting the method too). The
/// request line the host sends wold is `webapp-call <method> <params>`; what the cap leaves of
/// [`crate::portal::MAX_LINE`] is room for the longest method name and the word before it.
pub const WEBAPP_MAX_PARAMS: usize = crate::portal::MAX_LINE - 256;

/// The longest wold lets one web app call run (seconds): the sheet's wait, [`MAX_DOCTOR_STEPS`]
/// of wol-admind's long steps (1800 s each), its reply margin (60 s) and the call's own 120 s.
/// The web app host waits longer than this, so wold always answers first, with its own sentence
/// (QA-100 H9-03). `wol_core::sysapp::call_deadline_secs` computes the same figure from
/// `wol_core::admin` and a test there keeps the two equal.
pub const WEBAPP_CALL_DEADLINE_SECS: u64 =
    APPROVAL_WAIT_SECS as u64 + MAX_DOCTOR_STEPS as u64 * 1800 + 60 + 120;

/// Hex digits of a SHA-256 shown in a sheet's title; the full digest is on the detail line.
pub const SHA_PREFIX: usize = 12;

/// The first [`SHA_PREFIX`] characters of `sha256` (all of it when shorter).
pub fn sha_prefix(sha256: &str) -> &str {
    sha256
        .char_indices()
        .nth(SHA_PREFIX)
        .map_or(sha256, |(i, _)| &sha256[..i])
}

impl ApprovalRequest {
    /// Sentence end for "<helper> wants to …".
    pub fn describe(&self) -> String {
        let onoff = |on: &bool| if *on { "on" } else { "off" };
        match self {
            Self::Wifi { on } => format!("turn Wi-Fi {}", onoff(on)),
            Self::WifiJoin { ssid, .. } => format!("join the Wi-Fi network “{ssid}”"),
            Self::Bluetooth { on } => format!("turn Bluetooth {}", onoff(on)),
            Self::BootEnvironment { name } => {
                format!("start from the restore point “{name}” next time")
            }
            Self::Packages { verb, names } => {
                let list = names.join(", ");
                match (verb.as_str(), names.is_empty()) {
                    ("install", _) => format!("install {list} for every account"),
                    ("remove", _) => format!("remove {list} from this computer"),
                    ("update-holding", false) => {
                        format!("update all system apps except {list}")
                    }
                    (_, true) => "update all system apps".into(),
                    _ => format!("update {list}"),
                }
            }
            Self::PackageFile {
                name,
                version,
                sha256,
                ..
            } => format!(
                "install the package “{name}” {version} (SHA-256 {}…), which WolOS has not \
                 reviewed; its install steps run as the administrator",
                sha_prefix(sha256)
            ),
            Self::SystemRollback => {
                "undo the last system update (the previous release starts next time)".into()
            }
            Self::SystemChannel { channel } => {
                format!("switch system updates to the “{channel}” channel")
            }
            Self::DoctorFix { title, steps, .. } => {
                let shown: Vec<String> = steps
                    .iter()
                    .map(|s| format!("wol admin {}", s.join(" ")))
                    .collect();
                format!("fix “{title}” ({})", shown.join("; "))
            }
            Self::Change { what, .. } => what.clone(),
        }
    }

    /// A Doctor fix's steps as wol-admind requests, when every one is a [`DOCTOR_ACTIONS`] action
    /// and there are 1 to [`MAX_DOCTOR_STEPS`] of them (None for any other request).
    pub fn doctor_steps(&self) -> Option<Vec<crate::admin::AdminRequest>> {
        let Self::DoctorFix { steps, .. } = self else {
            return None;
        };
        if steps.is_empty() || steps.len() > MAX_DOCTOR_STEPS {
            return None;
        }
        steps
            .iter()
            .map(|s| {
                let (name, args) = s.split_first()?;
                let action = crate::admin::AdminAction::parse(name)?;
                DOCTOR_ACTIONS
                    .contains(&action)
                    .then(|| crate::admin::AdminRequest {
                        action,
                        args: args.to_vec(),
                    })
            })
            .collect()
    }

    /// Needs the administrator password (runs through wol-admind in the shell).
    pub fn is_admin(&self) -> bool {
        !matches!(self, Self::Change { .. })
    }
}

/// The shell's reply to `approve`: one JSON line, then the connection closes.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "answer", rename_all = "kebab-case")]
pub enum Answer {
    /// User-level change: the helper may run it this time.
    AllowOnce,
    /// User-level change: run it; the shell saved that this helper may do this kind of change
    /// without asking.
    AllowAlways,
    /// Administrator change: the user typed the password and the shell ran it.
    Done,
    /// Administrator change: allowed, but it did not work.
    Failed {
        message: String,
    },
    Deny,
    /// No answer within [`APPROVAL_TIMEOUT_SECS`].
    TimedOut,
    /// Too many requests already waiting on screen.
    Busy,
}

impl Answer {
    pub fn line(&self) -> String {
        format!("{}\n", serde_json::to_string(self).unwrap_or_default())
    }

    /// None for an empty or unknown reply (an old shell closes without answering).
    pub fn parse(line: &str) -> Option<Self> {
        serde_json::from_str(line.trim()).ok()
    }
}

impl Approval {
    /// One socket line; JSON never contains a raw newline.
    pub fn line(&self) -> String {
        format!(
            "{APPROVE} {}\n",
            serde_json::to_string(self).unwrap_or_default()
        )
    }

    /// Payload after `approve `; None when malformed.
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str(json.trim()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drop_shelf_can_be_toggled() {
        assert!(PANELS.contains(&"shelf"));
        let mut names = PANELS.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), PANELS.len(), "each panel name once");
    }

    #[test]
    fn approval_wire_round_trip() {
        let a = Approval {
            helper: "Claude".into(),
            tool: "wifi off".into(),
            request: ApprovalRequest::Wifi { on: false },
        };
        let l = a.line();
        assert_eq!(
            l,
            "approve {\"helper\":\"Claude\",\"tool\":\"wifi off\",\"request\":{\"action\":\"wifi\",\"on\":false}}\n"
        );
        let json = l.strip_prefix("approve ").unwrap();
        assert_eq!(Approval::parse(json), Some(a));
        assert_eq!(Approval::parse("{\"helper\":1}"), None);
        let p = ApprovalRequest::Packages {
            verb: "install".into(),
            names: vec!["firefox".into()],
        };
        assert!(p.is_admin());
        assert_eq!(p.describe(), "install firefox for every account");
        let held = ApprovalRequest::Packages {
            verb: "update-holding".into(),
            names: vec!["firefox".into(), "gimp".into()],
        };
        assert!(held.is_admin());
        assert_eq!(
            held.describe(),
            "update all system apps except firefox, gimp"
        );
        let line = Approval {
            helper: "/usr/bin/wopkg".into(),
            tool: "wopkg packages-install".into(),
            request: p.clone(),
        }
        .line();
        assert_eq!(
            Approval::parse(line.strip_prefix("approve ").unwrap_or("")).map(|a| a.request),
            Some(p)
        );
        let j = ApprovalRequest::WifiJoin {
            ssid: "Home\nNet".into(),
            password: None,
        };
        let line = Approval {
            request: j.clone(),
            helper: "x".into(),
            tool: "wifi join".into(),
        }
        .line();
        assert_eq!(
            line.matches('\n').count(),
            1,
            "one line even with \\n in the SSID"
        );
        assert_eq!(j.describe(), "join the Wi-Fi network “Home\nNet”");
        assert_eq!(
            ApprovalRequest::BootEnvironment {
                name: "wol-1".into()
            }
            .describe(),
            "start from the restore point “wol-1” next time"
        );
    }

    /// A user-level change carries its own sentence and capability; only admin requests need the
    /// password.
    #[test]
    fn change_requests_are_not_admin() {
        let c = ApprovalRequest::Change {
            capability: "appearance".into(),
            what: "switch to Dark appearance".into(),
        };
        assert_eq!(c.describe(), "switch to Dark appearance");
        assert!(!c.is_admin());
        assert!(ApprovalRequest::Wifi { on: true }.is_admin());
        let a = Approval {
            helper: "Claude".into(),
            tool: "appearance set".into(),
            request: c,
        };
        assert_eq!(
            Approval::parse(a.line().strip_prefix("approve ").unwrap()),
            Some(a)
        );
    }

    /// Audit C7-1: what wold used to run itself after a plain Allow is an administrator sheet,
    /// and its wire form is stable.
    #[test]
    fn root_package_changes_need_the_password() {
        let sha = "0123456789abcdef".repeat(4);
        let file = ApprovalRequest::PackageFile {
            staged:
                "/var/cache/wolos/install/0123456789abcdef0123456789abcdef/x-1-1-any.pkg.tar.zst"
                    .into(),
            name: "x".into(),
            version: "1:2.0-1".into(),
            sha256: sha.clone(),
        };
        let back = ApprovalRequest::SystemRollback;
        let chan = ApprovalRequest::SystemChannel {
            channel: "beta".into(),
        };
        for r in [&file, &back, &chan] {
            assert!(r.is_admin(), "{r:?}");
            let a = Approval {
                helper: "/usr/bin/wopkg".into(),
                tool: "wopkg packages-update".into(),
                request: r.clone(),
            };
            assert_eq!(
                Approval::parse(a.line().strip_prefix("approve ").unwrap()),
                Some(a)
            );
        }
        // ADR 0034: the sheet names what the staged copy contains, not the file name.
        assert!(
            file.describe()
                .starts_with("install the package “x” 1:2.0-1 (SHA-256 0123456789ab…)"),
            "{}",
            file.describe()
        );
        assert!(
            !file.describe().contains(&sha),
            "only a prefix in the title"
        );
        assert_eq!(sha_prefix("abc"), "abc");
        // A sheet from a wold that does not send the facts does not parse (fail closed).
        let old =
            r#"{"action":"package-file","staged":"/var/cache/wolos/install/n/x.pkg.tar.zst"}"#;
        assert!(serde_json::from_str::<ApprovalRequest>(old).is_err());
        assert!(file.describe().contains("as the administrator"));
        assert!(!file.describe().contains("/var/cache"));
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            "{\"action\":\"system-rollback\"}"
        );
        assert_eq!(
            chan.describe(),
            "switch system updates to the “beta” channel"
        );
    }

    /// A Doctor fix is an administrator sheet that names every step; only Doctor's actions parse
    /// into requests.
    #[test]
    fn a_doctor_fix_names_its_steps_and_carries_only_doctor_actions() {
        let f = ApprovalRequest::DoctorFix {
            id: "security.firewall".into(),
            title: "Turn on the firewall".into(),
            steps: vec![vec!["firewall".into(), "on".into()]],
        };
        assert!(f.is_admin());
        assert_eq!(
            f.describe(),
            "fix “Turn on the firewall” (wol admin firewall on)"
        );
        let a = Approval {
            helper: "org.wolos.webapp.doctor".into(),
            tool: "doctor_fix".into(),
            request: f.clone(),
        };
        assert_eq!(
            Approval::parse(a.line().strip_prefix("approve ").unwrap()),
            Some(a)
        );
        assert_eq!(
            f.doctor_steps(),
            Some(vec![crate::admin::AdminRequest {
                action: crate::admin::AdminAction::Firewall,
                args: vec!["on".into()],
            }])
        );
        let with = |steps: Vec<Vec<&str>>| ApprovalRequest::DoctorFix {
            id: "x.y".into(),
            title: "t".into(),
            steps: steps
                .into_iter()
                .map(|s| s.into_iter().map(String::from).collect())
                .collect(),
        };
        for bad in [
            vec![],
            vec![vec![]],
            vec![vec!["user-add", "eve"]],
            vec![vec!["firewall", "on"], vec!["nope"]],
            vec![vec!["pkg-upgrade"]; MAX_DOCTOR_STEPS + 1],
        ] {
            assert_eq!(with(bad.clone()).doctor_steps(), None, "{bad:?}");
        }
        assert_eq!(ApprovalRequest::SystemRollback.doctor_steps(), None);
    }

    /// The shell answers on the same connection: one JSON line.
    #[test]
    fn answer_wire_round_trip() {
        for (a, line) in [
            (Answer::AllowOnce, "{\"answer\":\"allow-once\"}\n"),
            (Answer::AllowAlways, "{\"answer\":\"allow-always\"}\n"),
            (Answer::Deny, "{\"answer\":\"deny\"}\n"),
            (Answer::Done, "{\"answer\":\"done\"}\n"),
            (Answer::TimedOut, "{\"answer\":\"timed-out\"}\n"),
            (Answer::Busy, "{\"answer\":\"busy\"}\n"),
            (
                Answer::Failed {
                    message: "no\nway".into(),
                },
                "{\"answer\":\"failed\",\"message\":\"no\\nway\"}\n",
            ),
        ] {
            assert_eq!(a.line(), line);
            assert_eq!(Answer::parse(line), Some(a));
        }
        assert_eq!(Answer::parse(""), None, "no reply = no answer");
        assert_eq!(Answer::parse("{\"answer\":\"yes\"}"), None);
        const { assert!(APPROVAL_TIMEOUT_SECS < APPROVAL_WAIT_SECS) };
    }
}
