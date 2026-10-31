//! Consent database for Settings > Privacy: which app may use which privileged protocol or WolOS
//! service. Same file and shape the compositor (`realtime::policy::Consent`) and wold (libwol
//! services, ADR 0020) read:
//! `{"grants":[{"app":"<canonical path>","capability":"<name>"}]}`. Malformed = no grants.
use crate::system::Host;
use serde_json::{Map, Value, json};

pub const FILE: &str = "/etc/wolos/consent.json";

/// What Settings may grant. The shell's own layers come from the system defaults, never from here.
pub const CAPABILITIES: [&str; 16] = [
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
];

const MAX_APP: usize = 1024;

/// Canonical absolute path (no empty, `.` or `..` segment, no control characters) and a capability
/// from [`CAPABILITIES`].
pub fn is_valid_grant(app: &str, capability: &str) -> bool {
    CAPABILITIES.contains(&capability)
        && app.len() <= MAX_APP
        && app.strip_prefix('/').is_some_and(|rest| {
            rest.split('/')
                .all(|s| !s.is_empty() && s != "." && s != "..")
        })
        && !app.chars().any(char::is_control)
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
    fn read_lists_grants() {
        let h = FakeHost::new(true);
        assert!(read(&h).is_empty());
        h.file(FILE, SHIPPED);
        assert_eq!(read(&h).len(), 2);
    }
}
