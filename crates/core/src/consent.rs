// SPDX-License-Identifier: BSD-2-Clause
//! Consent database for Settings > Privacy: which app may use which privileged protocol or WolOS
//! service: `{"grants":[{"app":"<canonical path>","capability":"<name>"}]}`. Malformed = no grants.
//! wold (libwol services, ADR 0020) reads the same file and shape. The compositor never reads it:
//! its rules (`crate::security::hypr_permissions`) are static, let only the system's own tools
//! copy the screen and send every app through the portal, which asks which screen to share.
use crate::system::Host;
use serde_json::{Map, Value, json};

pub const FILE: &str = "/etc/wolos/consent.json";

/// Screen recording is never a grant here: the compositor's rules are static and apps share the
/// screen through the portal, which asks each time. [`is_valid_grant`] refuses it, [`parse`]
/// ignores old entries, so the next [`set`] drops them from the file.
pub const SCREEN_CAPTURE: &str = "screen-capture";

/// Why `capability` can never be granted in this file, or None.
pub fn refusal(capability: &str) -> Option<&'static str> {
    (capability == SCREEN_CAPTURE).then_some(
        "screen-capture is not a grant: apps ask each time they share the screen, and the person chooses which screen or window",
    )
}

/// What Settings may grant. The shell's own layers come from the system defaults, never from here.
pub const CAPABILITIES: [&str; 25] = [
    "notifications",
    "input-injection",
    "clipboard-history",
    "window-list",
    // System packages through wold (ADR 0021, `shared::packages::GRANTS`).
    "packages-install",
    "packages-remove",
    "packages-update",
    "packages-unreviewed",
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
    "share",
    "background",
    "trash",
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
/// Entries no file may hold ([`refusal`]) are left out.
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
                .filter(|g| refusal(&g.capability).is_none())
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

/// What wold refused an account's apps (`<ASKED_DIR>/<uid>.json`, `{"asked":[{"app":"<canonical
/// path>","capability":"<name>"}]}`, newest last). No grant is still no access (ADR 0040: a
/// never-seen app is off); the list only lets Settings > Privacy & Security show the app with its
/// switch off, where every refusal sends the person. Written by wold alone (the caller's identity
/// from the kernel), owned by the account, mode 0600: nobody else sees which apps it ran.
pub const ASKED_DIR: &str = "/var/lib/wolos/consent-asked";
/// Apps kept per account; the oldest goes first.
pub const MAX_ASKED: usize = 64;
/// Bytes of an asked file anyone reads: the account owns its file and could grow it.
pub const MAX_ASKED_BYTES: u64 = 64 * 1024;

/// The account's asked file.
pub fn asked_path(uid: u32) -> std::path::PathBuf {
    std::path::Path::new(ASKED_DIR).join(format!("{uid}.json"))
}

/// (app, capability) pairs a Settings switch can turn on: valid grants (never screen capture),
/// at most [`MAX_ASKED`], newest last. Anything else in the file is left out.
pub fn asked(json: &str) -> Vec<(String, String)> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct File {
        #[serde(default)]
        asked: Vec<Grant>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Grant {
        app: String,
        capability: String,
    }
    let mut v: Vec<(String, String)> = serde_json::from_str::<File>(json)
        .map(|f| f.asked)
        .unwrap_or_default()
        .into_iter()
        .filter(|g| is_valid_grant(&g.app, &g.capability) && refusal(&g.capability).is_none())
        .map(|g| (g.app, g.capability))
        .collect();
    let mut seen = std::collections::HashSet::new();
    v.retain(|g| seen.insert(g.clone()));
    let extra = v.len().saturating_sub(MAX_ASKED);
    v.drain(..extra);
    v
}

/// `current` with `app` asking for `capability` recorded; None when there is nothing to write
/// (already listed, or not a grant Settings could turn on), so a refused app that retries never
/// rewrites the file.
pub fn note_asked(current: Option<&str>, app: &str, capability: &str) -> Option<String> {
    if !is_valid_grant(app, capability) || refusal(capability).is_some() {
        return None;
    }
    let mut v = current.map(asked).unwrap_or_default();
    if v.iter().any(|(a, c)| a == app && c == capability) {
        return None;
    }
    v.push((app.to_owned(), capability.to_owned()));
    let extra = v.len().saturating_sub(MAX_ASKED);
    v.drain(..extra);
    let list: Vec<Value> = v
        .iter()
        .map(|(a, c)| json!({"app": a, "capability": c}))
        .collect();
    let mut s = serde_json::to_string_pretty(&json!({ "asked": list })).unwrap_or_default();
    s.push('\n');
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    const SHIPPED: &str = r#"{"grants":[{"app":"/usr/bin/hello","capability":"notifications"},{"app":"/usr/bin/slurp","capability":"shell-layer"}],"_note":"dev VM"}"#;

    /// MANUAL 0.12.0: a new app's notification was refused and Privacy & Security had no row for
    /// it. A refusal is remembered once per (app, capability) so Settings can show the switch.
    #[test]
    fn a_refused_app_is_remembered_once() {
        let one = note_asked(None, "/home/a/Work/hello/hello", "notifications").unwrap();
        assert_eq!(
            asked(&one),
            [("/home/a/Work/hello/hello".into(), "notifications".into())]
        );
        assert_eq!(
            note_asked(Some(&one), "/home/a/Work/hello/hello", "notifications"),
            None
        );
        let two = note_asked(Some(&one), "/home/a/Work/hello/hello", "share").unwrap();
        assert_eq!(asked(&two).len(), 2);
        // Not a grant Settings can turn on: nothing to write.
        for (app, cap) in [
            ("hello", "notifications"),
            ("/a/../b", "notifications"),
            ("/usr/bin/x", "screen-capture"),
            ("/usr/bin/x", "made-up"),
        ] {
            assert_eq!(note_asked(None, app, cap), None, "{app} {cap}");
        }
        assert!(asked("not json").is_empty());
        let odd = concat!(
            r#"{"asked":[{"app":"rel","capability":"notifications"},"#,
            r#"{"app":"/x","capability":"screen-capture"},"#,
            r#"{"app":"/x","capability":"share"},{"app":"/x","capability":"share"}]}"#
        );
        assert_eq!(asked(odd), [("/x".into(), "share".into())]);
        // Only the documented shape: a file with other keys is no list.
        assert!(asked(r#"{"asked":[],"grants":[]}"#).is_empty());
    }

    #[test]
    fn the_asked_list_keeps_the_newest() {
        let mut t: Option<String> = None;
        for i in 0..MAX_ASKED + 5 {
            t = note_asked(t.as_deref(), &format!("/usr/bin/app{i}"), "notifications").or(t);
        }
        let v = asked(t.as_deref().unwrap());
        assert_eq!(v.len(), MAX_ASKED);
        assert_eq!(v[0].0, "/usr/bin/app5");
        assert_eq!(
            v.last().unwrap().0,
            format!("/usr/bin/app{}", MAX_ASKED + 4)
        );
        assert_eq!(
            asked_path(1000),
            std::path::Path::new(ASKED_DIR).join("1000.json")
        );
    }

    #[test]
    fn grants_parse_fail_closed() {
        assert_eq!(
            parse(SHIPPED),
            [
                ("/usr/bin/hello".into(), "notifications".into()),
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
        let off = set(Some(SHIPPED), "/usr/bin/hello", "notifications", false);
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
            ("/usr/bin/wayvnc", "screen-capture", false),
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
    fn screen_capture_is_refused_ignored_and_tidied() {
        assert!(refusal("screen-capture").is_some());
        assert!(refusal("notifications").is_none());
        assert!(!CAPABILITIES.contains(&SCREEN_CAPTURE));
        assert!(!is_valid_grant("/usr/bin/obs", "screen-capture"));
        // An old file with a screen-capture entry: read without it, and the next write drops it.
        let old = r#"{"grants":[{"app":"/usr/bin/obs","capability":"screen-capture"},{"app":"/usr/bin/hello","capability":"notifications"}]}"#;
        assert_eq!(
            parse(old),
            [("/usr/bin/hello".into(), "notifications".into())]
        );
        let next = set(Some(old), "/usr/bin/wlrctl", "window-list", true);
        assert!(!next.contains("screen-capture"), "{next}");
        assert_eq!(parse(&next).len(), 2);
        let h = FakeHost::new(true);
        h.file(FILE, old);
        assert_eq!(read(&h).len(), 1);
    }

    #[test]
    fn a_peer_grant_names_one_app() {
        let g = peer_grant("/usr/bin/notes");
        assert_eq!(peer_of(&g), Some("/usr/bin/notes"));
        assert!(is_valid_grant("/usr/bin/mail", &g));
        for bad in [
            "ipc:",
            "ipc:notes",
            "ipc:/usr/../etc/x",
            "ipc:/a\nb",
            "other:/a",
        ] {
            assert!(!is_valid_grant("/usr/bin/mail", bad), "{bad}");
        }
    }

    #[test]
    fn a_device_grant_names_one_device() {
        let g = device_grant(0x46d, 0xc52b);
        assert_eq!(g, "device:046d:c52b");
        assert_eq!(device_of(&g), Some("046d:c52b"));
        assert!(is_valid_grant("/usr/bin/mail", &g));
        for bad in [
            "device:",
            "device:46d:c52b",
            "device:046D:c52b",
            "device:046d:c52b:1",
        ] {
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
