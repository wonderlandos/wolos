//! `.wol` package file, index and paths (ADR 0011, archicture/modules/packages.md). Wire names and
//! paths are compatibility contract.

/// Archive members, in archive order: (name, required).
pub const MEMBERS: [(&str, bool); 5] = [
    (super::bundle::INFO_FILE, true),
    (ICON, false),
    (PAYLOAD, true),
    (MANIFEST, true),
    (SIGNATURE, false),
];
pub const ICON: &str = "icon.png";
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
    /// Icon URL (optional).
    #[serde(default)]
    pub icon: String,
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
        assert!(Index::parse(&entry(r#"{"screenshots":["x"]}"#)).is_ok());
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
