// SPDX-License-Identifier: GPL-2.0-or-later
//! Signature updates: a folder of data files plus `bundle.json`, signed with the same keys as a
//! release (`bundle.json.sig`, Ed25519, `wol_core::package::verify_index`). Nothing unsigned, older
//! or altered ever replaces what is installed.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const MANIFEST: &str = "bundle.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub name: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    pub version: u32,
    pub built: u64,
    pub files: Vec<Item>,
}

/// A manifest for the data files in `dir`.
pub fn describe(dir: &Path, version: u32, built: u64) -> Result<Bundle, String> {
    let mut files = Vec::new();
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut names: Vec<String> = rd
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| fits(n))
        .collect();
    names.sort();
    for name in names {
        let d = std::fs::read(dir.join(&name)).map_err(|e| e.to_string())?;
        files.push(Item {
            name,
            sha256: crate::hexs(&Sha256::digest(&d)),
        });
    }
    Ok(Bundle {
        version,
        built,
        files,
    })
}

/// A plain file name of a data file: nothing can land outside the folder.
fn fits(n: &str) -> bool {
    !n.contains('/')
        && !n.starts_with('.')
        && (n.ends_with(".xgdb") || n.ends_with(".yxc") || n == "NOTICES")
}

fn installed_version(dest: &Path) -> u32 {
    std::fs::read(dest.join(MANIFEST))
        .ok()
        .and_then(|t| serde_json::from_slice::<Bundle>(&t).ok())
        .map_or(0, |b| b.version)
}

/// Verify the bundle in `src` and swap it in at `dest`. Returns the new version.
pub fn install(src: &Path, dest: &Path, keys: &[[u8; 32]]) -> Result<u32, String> {
    let text = std::fs::read(src.join(MANIFEST)).map_err(|e| e.to_string())?;
    let sig = std::fs::read(src.join(format!("{MANIFEST}.sig"))).map_err(|e| e.to_string())?;
    match wol_core::package::verify_index(&text, Some(&sig), keys)? {
        wol_core::package::Trust::Signed(_) => {}
        wol_core::package::Trust::Unsigned => return Err("not signed".into()),
    }
    let b: Bundle = serde_json::from_slice(&text).map_err(|e| e.to_string())?;
    if b.version <= installed_version(dest) {
        return Err("not newer than what is installed".into());
    }
    let new = dest.with_extension("new");
    let _ = std::fs::remove_dir_all(&new);
    std::fs::create_dir_all(&new).map_err(|e| e.to_string())?;
    for f in &b.files {
        if !fits(&f.name) {
            return Err(format!("{}: not a signature file", f.name));
        }
        let d = std::fs::read(src.join(&f.name)).map_err(|e| e.to_string())?;
        if crate::hexs(&Sha256::digest(&d)) != f.sha256 {
            let _ = std::fs::remove_dir_all(&new);
            return Err(format!("{}: does not match its signed hash", f.name));
        }
        std::fs::write(new.join(&f.name), &d).map_err(|e| e.to_string())?;
    }
    std::fs::write(new.join(MANIFEST), &text).map_err(|e| e.to_string())?;
    let old = dest.with_extension("old");
    let _ = std::fs::remove_dir_all(&old);
    if dest.exists() {
        std::fs::rename(dest, &old).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&new, dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(b.version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wol_core::package::{public_of, sign};

    struct Fix {
        base: std::path::PathBuf,
        key: [u8; 32],
        seed: [u8; 32],
    }

    impl Fix {
        fn new(tag: &str) -> Self {
            let base =
                std::env::temp_dir().join(format!("xguard-bundle-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(base.join("src")).unwrap();
            let seed = [5; 32];
            Self {
                key: public_of(&seed),
                seed,
                base,
            }
        }
        fn src(&self) -> std::path::PathBuf {
            self.base.join("src")
        }
        fn dest(&self) -> std::path::PathBuf {
            self.base.join("rules")
        }
        fn publish(&self, version: u32, content: &[u8]) {
            std::fs::write(self.src().join("a.xgdb"), content).unwrap();
            let b = describe(&self.src(), version, 1).unwrap();
            let t = serde_json::to_vec(&b).unwrap();
            std::fs::write(self.src().join(MANIFEST), &t).unwrap();
            std::fs::write(self.src().join("bundle.json.sig"), sign(&self.seed, &t)).unwrap();
        }
    }

    #[test]
    fn a_signed_bundle_goes_in_and_a_newer_one_replaces_it() {
        let f = Fix::new("ok");
        f.publish(3, b"one");
        assert_eq!(install(&f.src(), &f.dest(), &[f.key]), Ok(3));
        assert_eq!(std::fs::read(f.dest().join("a.xgdb")).unwrap(), b"one");
        f.publish(4, b"two");
        assert_eq!(install(&f.src(), &f.dest(), &[f.key]), Ok(4));
        assert_eq!(std::fs::read(f.dest().join("a.xgdb")).unwrap(), b"two");
    }

    #[test]
    fn unsigned_old_or_altered_bundles_change_nothing() {
        let f = Fix::new("bad");
        f.publish(3, b"one");
        install(&f.src(), &f.dest(), &[f.key]).unwrap();
        // Rollback.
        f.publish(3, b"evil");
        assert!(install(&f.src(), &f.dest(), &[f.key]).is_err());
        // Wrong key.
        f.publish(9, b"evil");
        assert!(install(&f.src(), &f.dest(), &[[1; 32]]).is_err());
        // Data swapped after signing.
        f.publish(9, b"good");
        std::fs::write(f.src().join("a.xgdb"), b"evil").unwrap();
        assert!(install(&f.src(), &f.dest(), &[f.key]).is_err());
        // No signature.
        std::fs::remove_file(f.src().join("bundle.json.sig")).unwrap();
        assert!(install(&f.src(), &f.dest(), &[f.key]).is_err());
        assert_eq!(std::fs::read(f.dest().join("a.xgdb")).unwrap(), b"one");
        assert_eq!(installed_version(&f.dest()), 3);
    }

    #[test]
    fn a_manifest_cannot_write_outside_the_folder() {
        let f = Fix::new("path");
        let b = Bundle {
            version: 2,
            built: 1,
            files: vec![Item {
                name: "../evil.xgdb".into(),
                sha256: String::new(),
            }],
        };
        let t = serde_json::to_vec(&b).unwrap();
        std::fs::write(f.src().join(MANIFEST), &t).unwrap();
        std::fs::write(f.src().join("bundle.json.sig"), sign(&f.seed, &t)).unwrap();
        assert!(install(&f.src(), &f.dest(), &[f.key]).is_err());
    }
}
