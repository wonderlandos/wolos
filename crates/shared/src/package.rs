// SPDX-License-Identifier: BSD-2-Clause
//! `.wol` package file, index and paths (ADR 0011, docs/modules/packages.md). Wire names and
//! paths are compatibility contract.

/// Archive members, in archive order: (name, required). Between [`ICON`] and [`PAYLOAD`] a
/// package from the hub may carry up to [`MAX_SCREENSHOTS`] listing pictures, see
/// [`is_screenshot_member`].
pub const MEMBERS: [(&str, bool); 5] = [
    (super::bundle::INFO_FILE, true),
    (ICON, false),
    (PAYLOAD, true),
    (MANIFEST, true),
    (SIGNATURE, false),
];
pub const ICON: &str = "icon.png";
/// Prefix of the listing screenshots a package from the hub may carry (ADR 0047, hubcheck
/// `facts`): `screenshots/<name>.png|webp`, after [`ICON`] and before [`PAYLOAD`], each name once.
pub const SCREENSHOT_PREFIX: &str = "screenshots/";
/// Most screenshot members in one package.
pub const MAX_SCREENSHOTS: usize = 8;
/// Largest screenshot member (bytes); the hub's own limit for a listing picture.
pub const MAX_SCREENSHOT_SIZE: u64 = 2 << 20;
/// `screenshots/<name>.png` or `.webp`, the name of 1 to 32 lowercase letters, digits, `-` and `_`.
pub fn is_screenshot_member(name: &str) -> bool {
    name.strip_prefix(SCREENSHOT_PREFIX).is_some_and(|f| {
        f.strip_suffix(".png")
            .or_else(|| f.strip_suffix(".webp"))
            .is_some_and(|stem| {
                (1..=32).contains(&stem.len())
                    && stem.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'
                    })
            })
    })
}
pub const PAYLOAD: &str = "payload.pkg";
/// `SHA256 (<member>) = <hex>` lines for every other member, sorted by name.
pub const MANIFEST: &str = "SHA256";
/// Raw 64-byte Ed25519 signature of the [`MANIFEST`] bytes.
pub const SIGNATURE: &str = "SHA256.sig";

pub const MIME: &str = "application/x-wolos-package";
/// Trusted Ed25519 public keys (`*.pub`, PEM SPKI).
pub const KEYS_DIR: &str = "/usr/share/wolos/keys";
/// Install records: `<id>/Info.json`, shipped inside each payload (pacman owns them, so what
/// pacman installed is the only source of truth).
pub const RECORDS_DIR: &str = "/usr/share/wolos/packages";
/// Prefix the payload (a pacman package) installs into; target entries are relative to it.
pub const PREFIX: &str = "/usr";
/// wol-admind's root-only copy of the file it verifies and installs.
pub const STAGING_DIR: &str = "/var/cache/wolos/install";
/// Largest `.wol` file accepted (bytes).
pub const MAX_SIZE: u64 = 4 << 30;
pub const INDEX_URL: &str = "https://wonderlandos.github.io/wol-packages/index.json";
/// Detached signature next to the index: [`INDEX_URL`] + this suffix.
pub const INDEX_SIG_SUFFIX: &str = ".sig";
pub const INDEX_FORMAT: u32 = 1;

/// App Store categories (`wol_core::store::Category` names).
pub const CATEGORIES: [&str; 4] = ["Create", "Work", "Play", "Develop"];

/// What an app declares it uses, shown before install (not enforced): (id, label).
pub const CAPABILITIES: [(&str, &str); 9] = [
    ("network", "Internet and local network"),
    ("files", "Your files"),
    ("camera", "Camera"),
    ("microphone", "Microphone"),
    ("location", "Location"),
    ("screen-capture", "Screen recording"),
    ("notifications", "Notifications"),
    ("background", "Runs in the background"),
    ("devices", "USB and other devices"),
];

/// pkg origin-free package name: `[A-Za-z0-9][A-Za-z0-9._+-]*`, 1..=64.
pub fn is_pkg_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(c))
}

/// Reverse-DNS app id: two or more `[A-Za-z0-9_-]+` segments joined by dots, max 128.
pub fn is_app_id(s: &str) -> bool {
    s.len() <= 128
        && s.split('.').count() >= 2
        && s.split('.').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

/// pkg version string: `[A-Za-z0-9]` first, then `[A-Za-z0-9._,+~-]`, max 64.
pub fn is_version(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || b"._,+~-".contains(c))
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Longest listing description (characters), the hub index's bound (hub-pipeline
/// `schema/index.schema.json`).
pub const MAX_DESCRIPTION: usize = 4000;
/// Where the hub's listing pictures are: `<PICTURES_URL>/<sha256>.<png|webp>`, served as
/// `image/png` / `image/webp` (the CDN types a file by its extension).
pub const PICTURES_URL: &str = "https://cdn.wolos.org/plugins/pictures";

/// A hub picture's file name as the index carries it: `<sha256>.png` or `<sha256>.webp`.
pub fn is_picture_file(s: &str) -> bool {
    s.strip_suffix(".png")
        .or_else(|| s.strip_suffix(".webp"))
        .is_some_and(is_sha256)
}

/// The CDN URL of a hub picture, from the file name the index gives.
pub fn picture_url(file: &str) -> Option<String> {
    is_picture_file(file).then(|| format!("{PICTURES_URL}/{file}"))
}

/// A listing description: at most [`MAX_DESCRIPTION`] characters, no control character but
/// tab and line breaks.
pub fn is_description(s: &str) -> bool {
    s.chars().count() <= MAX_DESCRIPTION
        && !s
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
}

/// One installable `.wol` in [`Index`].
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    pub version: String,
    pub package: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// pkg ABI, e.g. `FreeBSD:15:amd64`.
    pub abi: String,
    /// `https://…/<package>-<version>.wol`.
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// Icon (optional): a URL (the wol-packages index) or, in a hub index, the file name
    /// `<sha256>.<png|webp>` of a listing picture ([`Entry::icon_picture`], [`picture_url`]).
    #[serde(default)]
    pub icon: String,
    /// The package's own description (hub index; optional, empty = none): plain text, at most
    /// [`MAX_DESCRIPTION`] characters.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Listing screenshots (hub index; optional): at most [`MAX_SCREENSHOTS`] file names
    /// `<sha256>.<png|webp>` of pictures on the CDN ([`picture_url`]), in the package's order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screenshots: Vec<String>,
    #[serde(default)]
    pub signed: bool,
}

impl Entry {
    /// Download name: `<package>-<version>.wol`.
    pub fn file_name(&self) -> String {
        format!(
            "{}-{}.{}",
            self.package,
            self.version,
            super::bundle::EXTENSION
        )
    }

    /// The icon's file name when it names a hub picture (not a URL).
    pub fn icon_picture(&self) -> Option<&str> {
        is_picture_file(&self.icon).then_some(self.icon.as_str())
    }

    fn check(&self) -> Result<(), String> {
        let bad = |what: &str| Err(format!("{}: bad {what}", self.id));
        if !is_app_id(&self.id) {
            return bad("id");
        }
        if !is_pkg_name(&self.package) {
            return bad("package");
        }
        if !is_version(&self.version) {
            return bad("version");
        }
        if !is_sha256(&self.sha256) {
            return bad("sha256");
        }
        if !self.url.starts_with("https://") || !self.url.ends_with(".wol") {
            return bad("url");
        }
        if self
            .categories
            .iter()
            .any(|c| !CATEGORIES.contains(&c.as_str()))
        {
            return bad("category");
        }
        if self
            .capabilities
            .iter()
            .any(|c| !CAPABILITIES.iter().any(|k| k.0 == c.as_str()))
        {
            return bad("capability");
        }
        if !is_description(&self.description) {
            return bad("description");
        }
        if self.screenshots.len() > MAX_SCREENSHOTS
            || !self.screenshots.iter().all(|s| is_picture_file(s))
        {
            return bad("screenshots");
        }
        Ok(())
    }
}

/// `index.json`: every `.wol` the repository publishes.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    pub format: u32,
    pub packages: Vec<Entry>,
}

impl Index {
    pub fn parse(json: &str) -> Result<Self, String> {
        let i: Index = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if i.format != INDEX_FORMAT {
            return Err(format!("index format {} not supported", i.format));
        }
        for e in &i.packages {
            e.check()?;
        }
        Ok(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "4f1c6a0d2b3e5f7a9c8d0e1f2a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d";

    fn entry(over: &str) -> String {
        let mut v: serde_json::Value = serde_json::from_str(&format!(
            r#"{{"id":"io.github.adbeveridge.Raider","name":"File Shredder","summary":"Securely delete your files","version":"3.1.0","package":"raider","categories":["Work"],"capabilities":["files"],"abi":"FreeBSD:15:amd64","url":"https://github.com/wonderlandos/wol-packages/releases/download/raider-3.1.0/raider-3.1.0.wol","sha256":"{SHA}","size":4096,"signed":true}}"#
        ))
        .unwrap();
        let o: serde_json::Value = serde_json::from_str(over).unwrap();
        for (k, x) in o.as_object().unwrap() {
            v[k] = x.clone();
        }
        format!(r#"{{"format":1,"packages":[{v}]}}"#)
    }

    #[test]
    fn index_parse_and_validation() {
        let i = Index::parse(&entry("{}")).unwrap();
        let e = &i.packages[0];
        assert_eq!(
            (e.package.as_str(), e.size, e.signed),
            ("raider", 4096, true)
        );
        assert_eq!(e.file_name(), "raider-3.1.0.wol");
        // Unknown fields are ignored: newer indexes stay readable.
        assert!(Index::parse(&entry(r#"{"homepage":["x"]}"#)).is_ok());
        for bad in [
            r#"{"sha256":"abc"}"#,
            r#"{"sha256":"4F1C6A0D2B3E5F7A9C8D0E1F2A3B4C5D6E7F8091A2B3C4D5E6F708192A3B4C5D"}"#,
            r#"{"url":"http://example.org/raider.wol"}"#,
            r#"{"url":"https://example.org/raider.zip"}"#,
            r#"{"id":"../evil"}"#,
            r#"{"id":""}"#,
            r#"{"package":"-rf"}"#,
            r#"{"version":"1 2"}"#,
            r#"{"version":""}"#,
            r#"{"categories":["Games"]}"#,
            r#"{"capabilities":["root"]}"#,
        ] {
            assert!(Index::parse(&entry(bad)).is_err(), "{bad}");
        }
        assert!(Index::parse(r#"{"format":2,"packages":[]}"#).is_err());
        assert!(Index::parse("not json").is_err());
    }

    #[test]
    fn listing_description_and_pictures() {
        // An index made before the keys existed parses, with none of them.
        let old = Index::parse(&entry("{}")).unwrap();
        let e = &old.packages[0];
        assert!(e.description.is_empty() && e.screenshots.is_empty() && e.icon_picture().is_none());
        // The live wol-packages index names its icon by URL: still read, not a hub picture.
        let url = Index::parse(&entry(
            r#"{"icon":"https://wonderlandos.github.io/wol-packages/icons/r.png"}"#,
        ))
        .unwrap();
        assert_eq!(url.packages[0].icon_picture(), None);
        // The hub index (hub-pipeline schema/index.schema.json): description, icon and screenshots
        // as CDN file names <sha256>.<png|webp>.
        let sha = "95d648d3d175b7df65af3f36caacaacc448897768f22945b5637685a8d5242f6";
        let pic = &format!("{sha}.webp");
        let shot = "3e0b5d9543783976039d0ab01171bb8b08c8fdc113b2891f467f6d0e7e0667f8.png";
        let new = Index::parse(&entry(&format!(
            r#"{{"description":"Shreds files.\nTabs\tare fine.","icon":"{pic}","screenshots":["{shot}","{pic}"]}}"#
        )))
        .unwrap();
        let e = &new.packages[0];
        assert_eq!(e.description, "Shreds files.\nTabs\tare fine.");
        assert_eq!(e.icon_picture(), Some(pic.as_str()));
        assert_eq!(e.screenshots, [shot, pic.as_str()]);
        assert_eq!(
            picture_url(pic).as_deref(),
            Some(
                "https://cdn.wolos.org/plugins/pictures/95d648d3d175b7df65af3f36caacaacc448897768f22945b5637685a8d5242f6.webp"
            )
        );
        for bad in [
            sha,
            &format!("{sha}.gif"),
            "../x.png",
            &format!("{sha}.png/x"),
        ] {
            assert_eq!(picture_url(bad), None, "{bad}");
        }
        // A bare sha256 (no type) is not a picture name.
        let bare = Index::parse(&entry(&format!(r#"{{"icon":"{sha}"}}"#))).unwrap();
        assert_eq!(bare.packages[0].icon_picture(), None);
        // An old entry serializes as before: no empty new keys.
        let text = serde_json::to_string(&old.packages[0]).unwrap();
        assert!(!text.contains("description") && !text.contains("screenshots"));
        // The schema's bounds: 4000 characters of text, at most 8 picture file names.
        let long = "\u{e9}".repeat(MAX_DESCRIPTION);
        assert!(Index::parse(&entry(&format!(r#"{{"description":"{long}"}}"#))).is_ok());
        let nine = vec![pic; 9];
        for bad in [
            format!(r#"{{"description":"{long}x"}}"#),
            r#"{"description":"a\u0000b"}"#.to_string(),
            r#"{"description":"a\u001bb"}"#.to_string(),
            format!(
                r#"{{"screenshots":{}}}"#,
                serde_json::to_string(&nine).unwrap()
            ),
            format!(r#"{{"screenshots":["{}"]}}"#, pic.to_uppercase()),
            format!(r#"{{"screenshots":["{sha}"]}}"#),
            r#"{"screenshots":["https://cdn.wolos.org/plugins/pictures/x.png"]}"#.to_string(),
            r#"{"screenshots":"x"}"#.to_string(),
        ] {
            assert!(Index::parse(&entry(&bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn ids_and_versions() {
        for (id, ok) in [
            ("org.wolos.Fixture", true),
            ("io.github.a_b-c.App", true),
            ("App", false),
            (".a.b", false),
            ("a..b", false),
            ("a.b/c", false),
            ("a.b.", false),
        ] {
            assert_eq!(is_app_id(id), ok, "{id}");
        }
        for (v, ok) in [
            ("3.1.0", true),
            ("2026.1_5", true),
            ("44.6,1", true),
            ("1.0.b", true),
            ("", false),
            ("1 0", false),
            ("1/0", false),
            ("-1", false),
        ] {
            assert_eq!(is_version(v), ok, "{v}");
        }
    }
}
