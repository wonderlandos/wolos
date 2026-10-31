//! Consent database for Settings > Privacy: which app may use which privileged protocol or WolOS
//! service. Same file and shape the compositor (`realtime::policy::Consent`) and wold (libwol
//! services, ADR 0020) read:
//! `{"grants":[{"app":"<canonical path>","capability":"<name>"}]}`. Malformed = no grants.
use crate::system::Host;
use serde_json::{Map, Value, json};

pub const FILE: &str = "/etc/wolos/consent.json";

/// What Settings may grant. The shell's own layers come from the system defaults, never from here.
pub const CAPABILITIES: [&str; 19] = [
    "screen-capture",
    "notifications",
    "input-injection",
    "clipboard-history",
    "window-list",
    "clipboard-read",
    "clipboard-write",
    "app-preferences",
    "file-panels",
    "power-info",
    "keep-awake",
    "network-info",
    "audio-info",
    "audio-control",
    "search",
    "search-provider",
    "account-info",
    "ipc",
    "devices",
];

const MAX_APP: usize = 1024;
/// Before the app in a grant for one peer: the holder may send that app messages.
const PEER: &str = "ipc:";
/// Before the device in a grant for one USB device.
const DEVICE: &str = "device:";

/// The grant that lets an app send messages to `peer`.
pub fn peer_grant(peer: &str) -> String {
    format!("{PEER}{peer}")
}

/// The app a grant lets its holder send messages to.
pub fn peer_of(capability: &str) -> Option<&str> {
    capability.strip_prefix(PEER)
}

/// The grant that lets an app open USB device `vendor`:`product`.
pub fn device_grant(vendor: u32, product: u32) -> String {
    format!("{DEVICE}{vendor:04x}:{product:04x}")
}

/// The device a grant lets its holder open, as `vendor:product`.
pub fn device_of(capability: &str) -> Option<&str> {
    let d = capability.strip_prefix(DEVICE)?;
    let hex = |s: &[u8]| s.len() == 4 && s.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    d.split_once(':')
        .is_some_and(|(v, p)| hex(v.as_bytes()) && hex(p.as_bytes()))
        .then_some(d)
}

/// Canonical absolute path: no empty, `.` or `..` segment, no control characters.
fn is_app(app: &str) -> bool {
    app.len() <= MAX_APP
        && app.strip_prefix('/').is_some_and(|rest| {
            rest.split('/')
                .all(|s| !s.is_empty() && s != "." && s != "..")
        })
        && !app.chars().any(char::is_control)
}

/// An app, and a capability from [`CAPABILITIES`] or a grant for one peer app.
pub fn is_valid_grant(app: &str, capability: &str) -> bool {
    is_app(app)
        && (CAPABILITIES.contains(&capability)
            || peer_of(capability).is_some_and(is_app)
            || device_of(capability).is_some())
}

/// (app, capability) pairs; a file that is not exactly the documented shape grants nothing.
pub fn parse(json: &str) -> Vec<(String, String)> {
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        grants: Vec<Grant>,
    }
    #[derive(serde::Deserialize)]
    struct Grant {
        app: String,
        capability: String,
    }
    serde_json::from_str::<File>(json).map_or_else(
        |_| Vec::new(),
        |f| {
            f.grants
                .into_iter()
                .map(|g| (g.app, g.capability))
                .collect()
        },
    )
}

/// `current` with the one grant added (`allow`) or removed; other grants and keys kept, pretty JSON.
/// A malformed file grants nothing, so the edit starts it over.
pub fn set(current: Option<&str>, app: &str, capability: &str, allow: bool) -> String {
    let mut top: Map<String, Value> = current
        .and_then(|c| serde_json::from_str::<Value>(c).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let mut grants = current.map(parse).unwrap_or_default();
    grants.retain(|g| !(g.0 == app && g.1 == capability));
    if allow {
        grants.push((app.to_owned(), capability.to_owned()));
    }
    top.insert(
        "grants".into(),
        grants
            .iter()
            .map(|(a, c)| json!({"app": a, "capability": c}))
            .collect(),
    );
    let mut s = serde_json::to_string_pretty(&Value::Object(top)).unwrap_or_default();
    s.push('\n');
    s
}

/// Grants on this machine (Settings > Privacy lists them).
pub fn read(h: &dyn Host) -> Vec<(String, String)> {
    h.read_text(FILE).map(|t| parse(&t)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    const SHIPPED: &str = r#"{"grants":[{"app":"/usr/bin/grim","capability":"screen-capture"},{"app":"/usr/bin/slurp","capability":"shell-layer"}],"_note":"dev VM"}"#;

    #[test]
    fn grants_parse_fail_closed() {
        assert_eq!(
            parse(SHIPPED),
            [
                ("/usr/bin/grim".into(), "screen-capture".into()),
                ("/usr/bin/slurp".into(), "shell-layer".into())
            ]
        );
        for bad in [
            "",
            "{not json",
            r#"{"grants":{}}"#,
            r#"{"grants":[{"app":"/a"}]}"#,
            "[]",
        ] {
            assert!(parse(bad).is_empty(), "{bad}");
        }
        assert!(parse("{}").is_empty());
    }

    #[test]
    fn allow_and_deny_touch_one_grant() {
        let wl = "/usr/bin/wlrctl";
        let on = set(Some(SHIPPED), wl, "window-list", true);
        assert_eq!(parse(&on).len(), 3);
        assert!(
            on.contains("\"_note\": \"dev VM\""),
            "other keys kept: {on}"
        );
        assert!(on.ends_with("}\n"));
        assert_eq!(set(Some(&on), wl, "window-list", true), on, "no duplicate");
        let off = set(Some(&on), wl, "window-list", false);
        assert_eq!(parse(&off), parse(SHIPPED));
        let off = set(Some(SHIPPED), "/usr/bin/grim", "screen-capture", false);
        assert_eq!(
            parse(&off),
            [("/usr/bin/slurp".into(), "shell-layer".into())]
        );
        // A malformed file grants nothing; the edit starts it over with just this grant.
        assert_eq!(
            parse(&set(Some("{oops"), wl, "window-list", true)),
            [(wl.into(), "window-list".into())]
        );
        assert_eq!(parse(&set(None, wl, "window-list", false)), []);
    }

    #[test]
    fn what_settings_may_grant() {
        for (app, cap, ok) in [
            ("/usr/bin/wayvnc", "screen-capture", true),
            ("/usr/bin/wayvnc", "input-injection", true),
            ("/usr/bin/wl-paste", "clipboard-history", true),
            ("/usr/bin/wlrctl", "window-list", true),
            ("/usr/bin/wlrctl", "shell-layer", false),
            ("/usr/bin/wlrctl", "root", false),
            ("wlrctl", "window-list", false),
            ("/usr/bin/../../../home/eve/x", "window-list", false),
            ("/usr//bin/x", "window-list", false),
            ("/usr/./bin/x", "window-list", false),
            ("/usr/bin/", "window-list", false),
            ("/usr/bin/x\ny", "window-list", false),
            ("xwayland", "window-list", false),
            ("", "window-list", false),
        ] {
            assert_eq!(is_valid_grant(app, cap), ok, "{app:?} {cap}");
        }
        assert!(!is_valid_grant(
            &format!("/{}", "a".repeat(1024)),
            "window-list"
        ));
    }

    #[test]
    fn a_peer_grant_names_one_app() {
        let g = peer_grant("/usr/bin/notes");
        assert_eq!(peer_of(&g), Some("/usr/bin/notes"));
        assert!(is_valid_grant("/usr/bin/mail", &g));
        for bad in ["ipc:", "ipc:notes", "ipc:/usr/../etc/x", "ipc:/a\nb", "other:/a"] {
            assert!(!is_valid_grant("/usr/bin/mail", bad), "{bad}");
        }
    }

    #[test]
    fn a_device_grant_names_one_device() {
        let g = device_grant(0x46d, 0xc52b);
        assert_eq!(g, "device:046d:c52b");
        assert_eq!(device_of(&g), Some("046d:c52b"));
        assert!(is_valid_grant("/usr/bin/mail", &g));
        for bad in ["device:", "device:46d:c52b", "device:046D:c52b", "device:046d:c52b:1"] {
            assert!(!is_valid_grant("/usr/bin/mail", bad), "{bad}");
        }
    }

    #[test]
    fn read_lists_grants() {
        let h = FakeHost::new(true);
        assert!(read(&h).is_empty());
        h.file(FILE, SHIPPED);
        assert_eq!(read(&h).len(), 2);
    }
}
