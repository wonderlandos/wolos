//! `.wol` bundle `Info.json` schema (ADR 0003 in IaraOS). Wire names are compatibility contract.
use serde::{Deserialize, Serialize};

/// Order = resolver preference.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Abi {
    #[serde(rename = "freebsd-native")]
    FreeBsdNative,
    #[serde(rename = "linux")]
    Linux,
    #[serde(rename = "proton")]
    Proton,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub abi: Abi,
    pub arch: String,
    /// Path relative to the bundle directory.
    pub entry: String,
}

/// `<Name>.wol/Info.json`, also the `Info.json` of a `.wol` package file (ADR 0011).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub id: String,
    pub version: String,
    /// Display name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// Icon theme name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// [`crate::package::CATEGORIES`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Declared, shown before install: [`crate::package::CAPABILITIES`] ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// pkg name of the payload (package files only).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub package: String,
    pub targets: Vec<Target>,
}

pub const INFO_FILE: &str = "Info.json";
pub const EXTENSION: &str = "wol";

impl Info {
    pub fn parse(json: &str) -> Result<Self, String> {
        let i: Info = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if i.id.is_empty() || i.targets.is_empty() {
            return Err("bundle needs id and targets".into());
        }
        if i.targets
            .iter()
            .any(|t| t.entry.starts_with('/') || t.entry.split('/').any(|p| p == ".."))
        {
            return Err("entry must stay inside the bundle".into());
        }
        use crate::package::{CAPABILITIES, CATEGORIES, is_pkg_name};
        if let Some(c) = i
            .categories
            .iter()
            .find(|c| !CATEGORIES.contains(&c.as_str()))
        {
            return Err(format!("unknown category {c}"));
        }
        if let Some(c) = i
            .capabilities
            .iter()
            .find(|c| !CAPABILITIES.iter().any(|k| k.0 == c.as_str()))
        {
            return Err(format!("unknown capability {c}"));
        }
        if !i.package.is_empty() && !is_pkg_name(&i.package) {
            return Err(format!("bad package name {}", i.package));
        }
        Ok(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_roundtrip_and_validation() {
        let j = r#"{"id":"org.wol.app","version":"1.0","targets":[{"abi":"proton","arch":"amd64","entry":"bin/app.exe"}]}"#;
        let i = Info::parse(j).unwrap();
        assert_eq!(i.targets[0].abi, Abi::Proton);
        assert_eq!(serde_json::to_string(&i).unwrap(), j);
        for bad in [
            r#"{"id":"","version":"1","targets":[{"abi":"linux","arch":"amd64","entry":"a"}]}"#,
            r#"{"id":"a","version":"1","targets":[]}"#,
            r#"{"id":"a","version":"1","targets":[{"abi":"win","arch":"amd64","entry":"a"}]}"#,
            r#"{"id":"a","version":"1","targets":[{"abi":"linux","arch":"amd64","entry":"/bin/sh"}]}"#,
            r#"{"id":"a","version":"1","targets":[{"abi":"linux","arch":"amd64","entry":"../x"}]}"#,
        ] {
            assert!(Info::parse(bad).is_err(), "{bad}");
        }
        assert!(Abi::FreeBsdNative < Abi::Linux && Abi::Linux < Abi::Proton);
    }

    /// `.wol` package fields (ADR 0011): optional, omitted when empty, validated when present.
    #[test]
    fn package_fields() {
        let j = r#"{"id":"io.github.adbeveridge.Raider","version":"3.1.0","name":"File Shredder","summary":"Securely delete your files","icon":"io.github.adbeveridge.Raider","categories":["Work"],"capabilities":["files"],"package":"raider","targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/raider"}]}"#;
        let i = Info::parse(j).unwrap();
        assert_eq!(
            (i.name.as_str(), i.package.as_str(), i.capabilities.len()),
            ("File Shredder", "raider", 1)
        );
        assert_eq!(serde_json::to_string(&i).unwrap(), j);
        let t = r#""targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/x"}]"#;
        for (extra, ok) in [
            (r#""categories":["Play","Develop"],"#, true),
            (r#""categories":["Games"],"#, false),
            (r#""capabilities":["network","camera"],"#, true),
            (r#""capabilities":["root"],"#, false),
            (r#""package":"x y","#, false),
            (r#""package":"-f","#, false),
            (r#""package":"py312-foo","#, true),
        ] {
            let j = format!(r#"{{"id":"a.b","version":"1",{extra}{t}}}"#);
            assert_eq!(Info::parse(&j).is_ok(), ok, "{j}");
        }
    }
}
