// SPDX-License-Identifier: GPL-2.0-or-later
//! Contract for /usr/share/wolos/keys (audit A13): an installed system has the release and
//! package keys, so `.wol` and release signatures can verify (shared::package::KEYS_DIR,
//! wol_core::package::read_keys), and it has a revocation list in a fixed format. Both
//! desktop/overlay and files/ land in the same package root (wolos-system PKGBUILD `package()`),
//! so keys may live in either tree; this test reads both.
#[path = "support/siblings.rs"]
mod siblings;

use std::path::PathBuf;

const TREES: &[&str] = &["desktop/overlay", "files"];
const KEYS: &str = "usr/share/wolos/keys";
const REVOKED: &str = "files/usr/share/wolos/keys/revoked.list";

fn key_dirs() -> Vec<PathBuf> {
    TREES
        .iter()
        .map(|t| siblings::root().join(t).join(KEYS))
        .filter(|d| d.is_dir())
        .collect()
}

/// (hex key, reason) rows of the revocation list; panics with the line on a malformed row.
fn revoked() -> Vec<(String, String)> {
    let text = std::fs::read_to_string(siblings::root().join(REVOKED))
        .unwrap_or_else(|e| panic!("{REVOKED}: {e}"));
    text.lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (key, reason) = l
                .split_once(' ')
                .unwrap_or_else(|| panic!("{l}: no reason"));
            assert!(
                key.len() == 64
                    && key
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{l}: the key is 64 lowercase hex characters"
            );
            assert!(!reason.trim().is_empty(), "{l}: no reason");
            (key.to_owned(), reason.to_owned())
        })
        .collect()
}

fn hex(k: &[u8; 32]) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn an_installed_system_ships_signing_keys() {
    let mut keys = Vec::new();
    for d in key_dirs() {
        for e in std::fs::read_dir(&d).expect("keys dir").flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "pub") {
                let pem = std::fs::read_to_string(&p).unwrap_or_default();
                let k = wol_core::package::public_key(&pem)
                    .unwrap_or_else(|| panic!("{}: not an Ed25519 PEM key", p.display()));
                keys.push(k);
            }
        }
        assert_eq!(
            wol_core::package::read_keys(&d).len(),
            std::fs::read_dir(&d)
                .expect("keys dir")
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "pub"))
                .count(),
            "{}: read_keys loads every *.pub",
            d.display()
        );
    }
    assert!(keys.len() >= 2, "release and package keys: {keys:?}");
}

#[test]
fn the_revocation_list_parses_and_revokes_no_shipped_key() {
    let rows = revoked();
    let mut shipped = Vec::new();
    // keys/ and keys/hub (the Hub's plugin index key).
    let dirs = key_dirs()
        .into_iter()
        .flat_map(|d| [d.join(wol_core::package::HUB_KEYS_SUBDIR), d])
        .filter(|d| d.is_dir());
    for d in dirs {
        // Every .pub file, not read_keys (which already drops revoked keys).
        for e in std::fs::read_dir(&d).expect("keys dir").flatten() {
            let pem = std::fs::read_to_string(e.path()).unwrap_or_default();
            shipped.extend(wol_core::package::public_key(&pem).iter().map(hex));
        }
    }
    for (k, reason) in &rows {
        assert!(
            !shipped.contains(k),
            "{k} is revoked ({reason}) but its .pub is still shipped: remove the file"
        );
    }
}

#[test]
fn the_packaging_installs_both_trees() {
    let pkgbuild = siblings::root().join("wolos-system/PKGBUILD");
    let Ok(text) = std::fs::read_to_string(&pkgbuild) else {
        // The recipe lives in wolos-pkgbuilds; without that sibling this property is checked there.
        return;
    };
    assert!(
        text.contains("desktop/overlay"),
        "overlay keys are packaged"
    );
    assert!(text.contains("$_repo/files"), "files/ keys are packaged");
}

#[test]
fn read_keys_honours_the_shipped_revocation_list_format() {
    let text = std::fs::read_to_string(siblings::root().join(REVOKED)).unwrap_or_default();
    assert_eq!(
        wol_core::package::parse_revoked(&text).len(),
        revoked().len()
    );
    assert_eq!(wol_core::package::REVOKED_FILE, "revoked.list");
}

/// HUB-KEYDIR: the Hub's plugin index key ships in keys/hub, so it verifies plugins only: it is a
/// plugin key and never a system key (release feed, update manifest, .wol packages).
#[test]
fn the_hub_index_key_is_scoped_to_plugins() {
    use wol_core::package::{HUB_KEYS_SUBDIR, Purpose, public_key, read_keys_for};
    let d = siblings::root().join("desktop/overlay").join(KEYS);
    let pem = std::fs::read_to_string(d.join(HUB_KEYS_SUBDIR).join("hub-index-2026.pub"))
        .expect("keys/hub/hub-index-2026.pub");
    let hub = public_key(&pem).expect("an Ed25519 PEM key");
    assert!(
        !d.join("hub-index-2026.pub").exists(),
        "not in keys/ itself"
    );
    let system = read_keys_for(&d, Purpose::System);
    let plugins = read_keys_for(&d, Purpose::Plugins);
    assert!(!system.contains(&hub), "the hub key never signs the system");
    assert!(plugins.contains(&hub));
    assert!(
        system.iter().all(|k| plugins.contains(k)),
        "release keys still sign plugins"
    );
}
