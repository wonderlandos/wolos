// SPDX-License-Identifier: GPL-2.0-or-later
//! App preferences for libwol (ADR 0020): one root-owned file per account and executable, so an
//! app reads and writes only its own, whatever it asks for. The caller comes from the kernel and
//! the key is checked by `shared::portal`, so no message can reach another path.
use serde_json::{Value, json};
use shared::portal::{Answer, Code, MAX_PREFS};
use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const DIR: &str = "/var/lib/wolos/prefs";

pub struct Prefs {
    root: PathBuf,
    /// A set is a read and a write: one at a time.
    writing: Mutex<()>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self::at(DIR)
    }
}

type Map = BTreeMap<String, String>;

impl Prefs {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            writing: Mutex::default(),
        }
    }

    fn file(&self, uid: u32, app: &str) -> PathBuf {
        let name = wol_core::package::sha256_hex(app.as_bytes());
        self.root.join(uid.to_string()).join(format!("{name}.json"))
    }

    /// A missing or damaged file is an empty one.
    fn load(path: &Path) -> Map {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Written whole, then renamed: a crash leaves the old file or the new one.
    fn save(path: &Path, map: &Map) -> std::io::Result<()> {
        let dir = path.parent().ok_or(std::io::ErrorKind::NotFound)?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let tmp = path.with_extension("tmp");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(json!(map).to_string().as_bytes())?;
        f.sync_all()?;
        std::fs::rename(tmp, path)
    }

    pub fn get(&self, uid: u32, app: &str, key: &str) -> Answer {
        let map = Self::load(&self.file(uid, app));
        Answer::with(match map.get(key) {
            Some(v) => json!({ "value": v }),
            None => json!({}),
        })
    }

    pub fn keys(&self, uid: u32, app: &str) -> Answer {
        let map = Self::load(&self.file(uid, app));
        Answer::with(Value::from_iter(map.into_keys()))
    }

    fn change(
        &self,
        uid: u32,
        app: &str,
        f: impl FnOnce(&mut Map) -> Result<(), Answer>,
    ) -> Answer {
        let _one = self.writing.lock().unwrap_or_else(|p| p.into_inner());
        let path = self.file(uid, app);
        let mut map = Self::load(&path);
        if let Err(a) = f(&mut map) {
            return a;
        }
        match Self::save(&path, &map) {
            Ok(()) => Answer::ok(),
            Err(_) => Answer::err(Code::Unavailable, "Preferences could not be saved."),
        }
    }

    pub fn set(&self, uid: u32, app: &str, key: String, value: String) -> Answer {
        self.change(uid, app, |m| {
            if m.len() >= MAX_PREFS && !m.contains_key(&key) {
                return Err(Answer::err(
                    Code::InvalidArgument,
                    format!("An app keeps at most {MAX_PREFS} preferences."),
                ));
            }
            m.insert(key, value);
            Ok(())
        })
    }

    pub fn remove(&self, uid: u32, app: &str, key: &str) -> Answer {
        self.change(uid, app, |m| {
            m.remove(key);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wol-prefs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn each_app_of_each_account_has_its_own() {
        let root = temp("own");
        let p = Prefs::at(&root);
        assert!(p.set(1000, "/usr/bin/a", "k".into(), "one".into()).ok);
        assert!(p.set(1000, "/usr/bin/b", "k".into(), "two".into()).ok);
        assert!(p.set(1001, "/usr/bin/a", "k".into(), "three".into()).ok);
        let got = |u, a| p.get(u, a, "k").data.unwrap();
        assert_eq!(got(1000, "/usr/bin/a"), json!({"value": "one"}));
        assert_eq!(got(1000, "/usr/bin/b"), json!({"value": "two"}));
        assert_eq!(got(1001, "/usr/bin/a"), json!({"value": "three"}));
        assert_eq!(got(1002, "/usr/bin/a"), json!({}));
        let f = p.file(1000, "/usr/bin/a");
        let mode = std::fs::metadata(&f).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir = std::fs::metadata(f.parent().unwrap()).unwrap();
        assert_eq!(dir.permissions().mode() & 0o777, 0o700);
        assert!(p.remove(1000, "/usr/bin/a", "k").ok);
        assert_eq!(got(1000, "/usr/bin/a"), json!({}));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bounded_and_a_damaged_file_is_empty() {
        let root = temp("bound");
        let p = Prefs::at(&root);
        for i in 0..MAX_PREFS {
            assert!(p.set(1, "/a", format!("k{i}"), "v".into()).ok);
        }
        let a = p.set(1, "/a", "more".into(), "v".into());
        assert_eq!(a.code, Some(Code::InvalidArgument));
        assert!(p.set(1, "/a", "k0".into(), "new".into()).ok, "changing one");
        let keys = p.keys(1, "/a").data.unwrap();
        assert_eq!(keys.as_array().unwrap().len(), MAX_PREFS);
        std::fs::write(p.file(1, "/a"), "{broken").unwrap();
        assert_eq!(p.keys(1, "/a").data.unwrap(), json!([]));
        let _ = std::fs::remove_dir_all(root);
    }
}
