//! Shell socket contract: panel names `wol shell toggle|show|hide <panel>` accepts.
pub const PANELS: &[&str] = &[
    "switcher",
    "notifications",
    "session",
    "clipboard",
    "osd",
    "control-center",
    "overview",
    "screenshot",
    "switcher-prev",
    "screen-record",
    // Edit Widgets on the desktop (ADR 0010).
    "widgets",
    // ⌥⌘Esc picker.
    "force-quit",
    // `show` moves the keyboard into the bar or the Dock (⌃F2, ⌃F3), `hide` gives it back.
    "menubar",
    "dock",
    // `hide` dismisses every banner on screen (Esc).
    "banners",
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
    /// A user-level change the helper runs itself once allowed. `what` is the sentence end
    /// ("switch to Dark appearance"); `capability` is what Always Allow grants.
    Change {
        capability: String,
        what: String,
    },
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
            Self::Change { what, .. } => what.clone(),
        }
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
