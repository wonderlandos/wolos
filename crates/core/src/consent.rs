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

/// Grants the packages of bundled apps ship (vendor defaults): `*.json` files of the same
/// `{"grants":[...]}` shape in a root-owned folder under `/usr`, one per package. A bundled app
/// then works on a fresh system (the first Delete in Gallery moves the picture to the Trash
/// instead of showing a refusal), and the person can still switch any of them off in Settings >
/// Privacy & Security: the switch writes the pair to `"revoked"` in [`FILE`], which survives
/// package updates (a package never rewrites that file, so no file conflict either).
pub const DEFAULTS_DIR: &str = "/usr/share/wolos/consent.d";

/// What a package may grant by default: harmless verbs that ask the person each time or only act
/// on what the app is handed. Nothing that reads the screen, the keyboard, the clipboard history,
/// the window list or other apps' messages, and no package changes. A default outside this list
/// is ignored, so a third-party package cannot ship itself a powerful grant.
pub const DEFAULTABLE: [&str; 6] = [
    "trash",
    "share",
    "notifications",
    "file-panels",
    "app-preferences",
    "search-provider",
];

/// Most default files read, and bytes read from each (a package ships a few lines).
const MAX_DEFAULT_FILES: usize = 64;
const MAX_DEFAULT_BYTES: u64 = 16 * 1024;

/// Whether a package may ship `capability` for `app` as a default: a valid grant of a
/// [`DEFAULTABLE`] capability for a program directly in `/usr/bin`.
pub fn is_default_grant(app: &str, capability: &str) -> bool {
    is_valid_grant(app, capability)
        && DEFAULTABLE.contains(&capability)
        && app
            .strip_prefix("/usr/bin/")
            .is_some_and(|name| !name.contains('/'))
}

/// The grants of the default files' texts: only [`is_default_grant`] pairs, no duplicates.
pub fn parse_defaults<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for t in texts {
        for g in parse(t) {
            if is_default_grant(&g.0, &g.1) && !v.contains(&g) {
                v.push(g);
            }
        }
    }
    v
}

/// Vendor defaults on this machine ([`DEFAULTS_DIR`]).
pub fn defaults(h: &dyn Host) -> Vec<(String, String)> {
    let texts: Vec<String> = h
        .list_dir(DEFAULTS_DIR)
        .into_iter()
        .filter(|p| p.ends_with(".json"))
        .take(MAX_DEFAULT_FILES)
        .filter_map(|p| {
            // Bounded like `defaults_in`: a file past the limit is cut, no longer valid JSON, ignored.
            let mut buf = vec![0u8; MAX_DEFAULT_BYTES as usize];
            let n = h.read_head(&p, &mut buf)?;
            String::from_utf8(buf[..n].to_vec()).ok()
        })
        .collect();
    parse_defaults(texts.iter().map(String::as_str))
}

/// [`defaults`] for a process with no [`Host`] (wold, the administrator helper): reads `dir`,
/// bounded, nothing for a missing or unreadable folder.
pub fn defaults_in(dir: &std::path::Path) -> Vec<(String, String)> {
    use std::io::Read;
    let mut names: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    names.sort();
    let texts: Vec<String> = names
        .into_iter()
        .take(MAX_DEFAULT_FILES)
        .filter_map(|p| {
            let mut s = String::new();
            std::fs::File::open(p)
                .ok()?
                .take(MAX_DEFAULT_BYTES)
                .read_to_string(&mut s)
                .ok()?;
            Some(s)
        })
        .collect();
    parse_defaults(texts.iter().map(String::as_str))
}

/// The pairs the person switched off although a package grants them (`"revoked"` in [`FILE`]).
/// Same shape as `"grants"`; a file that is not that shape revokes nothing.
pub fn revoked(json: &str) -> Vec<(String, String)> {
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        revoked: Vec<Grant>,
    }
    #[derive(serde::Deserialize)]
    struct Grant {
        app: String,
        capability: String,
    }
    serde_json::from_str::<File>(json).map_or_else(
        |_| Vec::new(),
        |f| {
            f.revoked
                .into_iter()
                .map(|g| (g.app, g.capability))
                .collect()
        },
    )
}

/// Whether `text` is a consent file this module understands: a JSON object whose `grants` and
/// `revoked` keys, when present, are lists of `{app, capability}` strings. Other keys are free.
fn is_well_formed(text: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Pair {
        #[allow(dead_code)]
        app: String,
        #[allow(dead_code)]
        capability: String,
    }
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        #[allow(dead_code)]
        grants: Vec<Pair>,
        #[serde(default)]
        #[allow(dead_code)]
        revoked: Vec<Pair>,
    }
    serde_json::from_str::<Value>(text).is_ok_and(|v| v.is_object())
        && serde_json::from_str::<File>(text).is_ok()
}

/// What an app really holds: the vendor `defaults` and the grants of the file `file_text` (the
/// person's own choices), minus every pair in the file's `revoked` list. No duplicates.
///
/// Revoked wins over grants: a pair in both is not held, because switching something off is the
/// person's last word (Settings never writes both; a hand edit or an old entry might). No file at
/// all is a fresh system and the defaults stand alone. A file that is not well formed, as a whole
/// (not a JSON object, or a `grants` or `revoked` key that is not a list of pairs), grants nothing,
/// defaults included (fail closed, as [`parse`]): the next edit from Settings starts it over.
pub fn effective(file_text: Option<&str>, defaults: &[(String, String)]) -> Vec<(String, String)> {
    if file_text.is_some_and(|t| !is_well_formed(t)) {
        return Vec::new();
    }
    let off = file_text.map(revoked).unwrap_or_default();
    let mut v: Vec<(String, String)> = Vec::new();
    for g in defaults
        .iter()
        .cloned()
        .chain(file_text.map(parse).unwrap_or_default())
    {
        if !off.contains(&g) && !v.contains(&g) {
            v.push(g);
        }
    }
    v
}

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
/// A malformed file grants nothing, so the edit starts it over. Knows no vendor defaults: see
/// [`set_over`].
pub fn set(current: Option<&str>, app: &str, capability: &str, allow: bool) -> String {
    set_over(current, &[], app, capability, allow)
}

/// [`set`] over the vendor `defaults`. Switching a default off records it in `"revoked"` (the
/// default itself is the package's file and stays); switching it on again drops that entry, and
/// a default needs no entry in `"grants"`. Any other pair is added to or removed from `"grants"`
/// as in [`set`]. A `"revoked"` entry stays when its package goes away, so installing the app again
/// keeps the person's choice; only a default can be added to it.
pub fn set_over(
    current: Option<&str>,
    defaults: &[(String, String)],
    app: &str,
    capability: &str,
    allow: bool,
) -> String {
    let mut top: Map<String, Value> = current
        .and_then(|c| serde_json::from_str::<Value>(c).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let pair = (app.to_owned(), capability.to_owned());
    let is_default = defaults.contains(&pair);
    let mut grants = current.map(parse).unwrap_or_default();
    let mut off = current.map(revoked).unwrap_or_default();
    grants.retain(|g| *g != pair);
    off.retain(|r| *r != pair);
    if allow && !is_default {
        grants.push(pair);
    } else if !allow && is_default {
        off.push(pair);
    }
    let list = |v: &[(String, String)]| -> Value {
        v.iter()
            .map(|(a, c)| json!({"app": a, "capability": c}))
            .collect()
    };
    top.insert("grants".into(), list(&grants));
    if off.is_empty() {
        top.remove("revoked");
    } else {
        top.insert("revoked".into(), list(&off));
    }
    let mut s = serde_json::to_string_pretty(&Value::Object(top)).unwrap_or_default();
    s.push('\n');
    s
}

/// Grants on this machine (Settings > Privacy lists them): the vendor defaults the person has
/// not switched off, and the person's own ([`effective`]).
pub fn read(h: &dyn Host) -> Vec<(String, String)> {
    effective(h.read_text(FILE).as_deref(), &defaults(h))
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

    const GALLERY: &str = r#"{"grants":[{"app":"/usr/bin/wol-gallery","capability":"trash"},{"app":"/usr/bin/wol-gallery","capability":"share"}]}"#;

    fn gallery_defaults() -> Vec<(String, String)> {
        parse_defaults([GALLERY])
    }

    /// E2E-GALLERY: the first Delete in a bundled app on a fresh system showed a refusal because
    /// no grant existed. The package ships its two grants; a fresh system (no consent file) holds them.
    #[test]
    fn a_fresh_system_holds_the_defaults_of_bundled_apps() {
        let d = gallery_defaults();
        assert_eq!(d.len(), 2);
        let held = effective(None, &d);
        assert_eq!(held, d);
        // The person's own grants come after, without repeating a default.
        let file = set(None, "/usr/bin/wlrctl", "window-list", true);
        let held = effective(Some(&file), &d);
        assert_eq!(held.len(), 3);
        assert_eq!(held[2], ("/usr/bin/wlrctl".into(), "window-list".into()));
        let again = set_over(Some(&file), &d, "/usr/bin/wol-gallery", "trash", true);
        assert_eq!(
            effective(Some(&again), &d),
            held,
            "a default is not copied into the file"
        );
    }

    #[test]
    fn switching_a_default_off_survives_and_on_restores() {
        let d = gallery_defaults();
        let app = "/usr/bin/wol-gallery";
        let off = set_over(None, &d, app, "trash", false);
        assert_eq!(revoked(&off), [(app.into(), "trash".into())]);
        assert_eq!(effective(Some(&off), &d), [(app.into(), "share".into())]);
        // Another switch leaves the choice alone, and so does an update of the package (the same
        // defaults, a file only the person writes).
        let other = set_over(Some(&off), &d, "/usr/bin/wlrctl", "window-list", true);
        assert_eq!(revoked(&other).len(), 1);
        assert!(!effective(Some(&other), &d).contains(&(app.into(), "trash".into())));
        // Off twice is one entry; on again removes it and writes no grant of its own.
        let twice = set_over(Some(&off), &d, app, "trash", false);
        assert_eq!(revoked(&twice).len(), 1);
        let on = set_over(Some(&off), &d, app, "trash", true);
        assert!(revoked(&on).is_empty());
        assert!(!on.contains("revoked"), "{on}");
        assert!(parse(&on).is_empty());
        assert_eq!(effective(Some(&on), &d), d);
        // A pair that is no default is never recorded as revoked.
        let plain = set_over(Some(SHIPPED), &d, "/usr/bin/hello", "notifications", false);
        assert!(revoked(&plain).is_empty());
        assert_eq!(parse(&plain).len(), 1);
        // The old entry of a package that went away does not block a grant from the person.
        let stale = set_over(Some(&off), &[], app, "trash", true);
        assert!(revoked(&stale).is_empty());
        assert_eq!(parse(&stale), [(app.into(), "trash".into())]);
    }

    #[test]
    fn a_broken_file_grants_nothing_not_even_defaults() {
        let d = gallery_defaults();
        for bad in ["", "{oops", "[]", "7"] {
            assert!(effective(Some(bad), &d).is_empty(), "{bad}");
        }
        assert_eq!(effective(Some("{}"), &d), d);
        // A malformed key fails the whole file, defaults included: a switch-off that cannot be
        // read must not turn a default back on.
        for bad in [
            r#"{"grants":{}}"#,
            r#"{"grants":[{"app":"/usr/bin/a"}]}"#,
            r#"{"grants":[],"revoked":{}}"#,
            r#"{"grants":[],"revoked":[{"app":"/usr/bin/wol-gallery"}]}"#,
            r#"{"revoked":"trash"}"#,
        ] {
            assert!(effective(Some(bad), &d).is_empty(), "{bad}");
        }
        // Other keys are free.
        assert_eq!(effective(Some(r#"{"_note":"dev VM"}"#), &d), d);
    }

    #[test]
    fn revoked_wins_over_a_grant_of_the_same_pair() {
        let d = gallery_defaults();
        let app = "/usr/bin/wol-gallery";
        let both = format!(
            r#"{{"grants":[{{"app":"{app}","capability":"trash"}},{{"app":"/usr/bin/x","capability":"share"}}],
                "revoked":[{{"app":"{app}","capability":"trash"}},{{"app":"/usr/bin/x","capability":"share"}}]}}"#
        );
        let held = effective(Some(&both), &d);
        assert_eq!(held, [(app.into(), "share".into())], "{held:?}");
        // No default involved: the grant is switched off all the same.
        assert!(effective(Some(&both), &[]).is_empty());
    }

    #[test]
    fn a_default_file_past_the_limit_is_cut_and_ignored() {
        let h = FakeHost::linux();
        // The grant sits past the bound: cut off, no longer valid JSON, so nothing is granted.
        let late = format!(
            r#"{{"grants":[{}{{"app":"/usr/bin/wol-gallery","capability":"trash"}}]}}"#,
            " ".repeat(MAX_DEFAULT_BYTES as usize)
        );
        h.file(&format!("{DEFAULTS_DIR}/late.json"), &late);
        assert!(
            parse_defaults([late.as_str()]).len() == 1,
            "valid when read whole"
        );
        assert!(defaults(&h).is_empty());
        // Within the bound the same file is read.
        h.file(&format!("{DEFAULTS_DIR}/late.json"), GALLERY);
        assert_eq!(defaults(&h), gallery_defaults());
    }

    #[test]
    fn a_package_default_is_only_a_harmless_verb_for_a_program_in_usr_bin() {
        for (app, cap, ok) in [
            ("/usr/bin/wol-gallery", "trash", true),
            ("/usr/bin/wol-gallery", "share", true),
            ("/usr/bin/wol-notes", "search-provider", true),
            ("/usr/bin/x", "packages-install", false),
            ("/usr/bin/x", "clipboard-history", false),
            ("/usr/bin/x", "input-injection", false),
            ("/usr/bin/x", "window-list", false),
            ("/usr/bin/x", "ipc", false),
            ("/usr/bin/x", "screen-capture", false),
            ("/usr/bin/x", "made-up", false),
            ("/home/eve/x", "trash", false),
            ("/opt/x/x", "trash", false),
            ("/usr/bin/sub/x", "trash", false),
            ("/usr/bin/../../tmp/x", "trash", false),
            ("wol-gallery", "trash", false),
        ] {
            assert_eq!(is_default_grant(app, cap), ok, "{app} {cap}");
        }
        let text = r#"{"grants":[{"app":"/usr/bin/a","capability":"trash"},
            {"app":"/usr/bin/a","capability":"trash"},{"app":"/usr/bin/a","capability":"devices"}]}"#;
        assert_eq!(
            parse_defaults([text, "{bad"]),
            [("/usr/bin/a".into(), "trash".into())]
        );
        // Every defaultable name is a real capability.
        for c in DEFAULTABLE {
            assert!(CAPABILITIES.contains(&c), "{c}");
        }
    }

    #[test]
    fn defaults_come_from_the_json_files_of_the_folder() {
        let h = FakeHost::linux();
        h.file(&format!("{DEFAULTS_DIR}/org.wolos.gallery.json"), GALLERY);
        h.file(
            &format!("{DEFAULTS_DIR}/README"),
            GALLERY.replace("gallery", "other").as_str(),
        );
        assert_eq!(defaults(&h), gallery_defaults());
        h.file(
            FILE,
            &set_over(None, &defaults(&h), "/usr/bin/wol-gallery", "share", false),
        );
        assert_eq!(read(&h), [("/usr/bin/wol-gallery".into(), "trash".into())]);
        assert!(defaults(&FakeHost::linux()).is_empty());
        // The real reader gives the same on disk.
        let dir = std::env::temp_dir().join(format!("wol-consent-d-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("org.wolos.gallery.json"), GALLERY).unwrap();
        std::fs::write(dir.join("notes.txt"), GALLERY).unwrap();
        assert_eq!(defaults_in(&dir), gallery_defaults());
        assert!(defaults_in(&dir.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(dir);
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
