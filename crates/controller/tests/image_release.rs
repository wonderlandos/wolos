// SPDX-License-Identifier: GPL-2.0-or-later
//! Package and release pipeline: `packages` (build order + local deps; changed packages:
//! wolos-build lib/plan.py), the release manifest `wol update` reads and its
//! signature, and the CI workflows that build the `[wolos]` repo and the ISO.
#![allow(dead_code, unused_imports)]
#[path = "support/siblings.rs"]
mod siblings;

use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};
fn repo(p: &str) -> PathBuf {
    siblings::root()
        .join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-release-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// release-manifest.sh writes exactly what `wol update` parses.
#[test]
fn release_manifest_is_what_updates_read() {
    let d = scratch("manifest");
    let iso = d.join("wolos-0.11.2-x86_64.iso");
    fs::write(&iso, "iso bytes").unwrap();
    let o = Command::new("bash")
        .arg(repo("release/release-manifest.sh"))
        .args([
            iso.to_str().unwrap(),
            "2026-10-20",
            "r42",
            "https://example.org/img",
        ])
        .args(["0.11.2", "7.3.0-1"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = String::from_utf8(o.stdout).unwrap();
    let m = wol_core::update::check::Manifest::parse(&text).expect(&text);
    assert_eq!(m.version, "0.11.2");
    assert_eq!(m.snapshot, "2026/10/20");
    assert_eq!(m.wolos_repo, "r42");
    assert_eq!(m.kernel, "7.3.0-1");
    assert_eq!(m.image, "https://example.org/img/wolos-0.11.2-x86_64.iso");
    // VERSION and KERNEL come from the repo the image was built from (wolos-iso build.sh reads its
    // database), never from a source tree: without them the script refuses.
    let o = Command::new("bash")
        .arg(repo("release/release-manifest.sh"))
        .args([
            iso.to_str().unwrap(),
            "2026-10-20",
            "r42",
            "https://example.org/img",
        ])
        .output()
        .unwrap();
    assert!(!o.status.success(), "a manifest without VERSION and KERNEL");
}

/// sign-release.sh: raw Ed25519 over the file bytes, the signature `wol update` verifies.
#[test]
fn release_signature_verifies_with_the_package_keys() {
    let d = scratch("sign");
    let key = d.join("key.pem");
    let ok = Command::new("openssl")
        .args(["genpkey", "-algorithm", "ed25519", "-out"])
        .arg(&key)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("no openssl with ed25519: skipped");
        return;
    }
    let der = Command::new("openssl")
        .args(["pkey", "-pubout", "-outform", "DER", "-in"])
        .arg(&key)
        .output()
        .unwrap()
        .stdout;
    let public: [u8; 32] = der[der.len() - 32..].try_into().unwrap();
    let rel = d.join("release.json");
    fs::write(&rel, br#"{"version":"0.11.2"}"#).unwrap();
    let o = Command::new("bash")
        .arg(repo("release/sign-release.sh"))
        .arg(&rel)
        .arg(&key)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let sig = fs::read(d.join("release.json.sig")).unwrap();
    let t = wol_core::package::verify_index(&fs::read(&rel).unwrap(), Some(&sig), &[public]);
    assert!(
        matches!(t, Ok(wol_core::package::Trust::Signed(0))),
        "{t:?}"
    );
}

/// The check fetches from where the ISO workflow publishes.
#[test]
fn update_check_reads_the_published_release() {
    let check = read("files/usr/lib/wolos/update-check");
    let url =
        "https://github.com/wonderlandos/wolos-packages/releases/latest/download/release.json";
    assert!(check.contains(url), "update-check URL");
    let iso = read(".github/workflows/release.yml");
    for want in [
        "tags:",
        "'v*'",
        "workflow_dispatch",
        "archlinux",
        "mkarchiso",
        "release-manifest.sh",
        "sign-release.sh",
        "release.json",
        "release.json.sig",
        ".sha256",
        "concurrency:",
        "gh release create",
        "--latest",
    ] {
        assert!(iso.contains(want), "release.yml lacks {want}");
    }
    // Published in the releases repo the check reads (GH_REPO), not in this one.
    assert!(
        iso.contains("GH_REPO: ${{ vars.WOLOS_RELEASES_REPO"),
        "release.yml GH_REPO"
    );
    assert!(
        !iso.contains("action-gh-release"),
        "release.yml publishes with the action"
    );
}

/// The release key CI signs with (secret WOLOS_RELEASE_KEY) is one of the keys updates trust.
#[test]
fn image_trusts_the_ci_release_key() {
    let dir = repo("desktop/overlay/usr/share/wolos/keys");
    let keys = wol_core::package::read_keys(&dir);
    assert!(keys.len() >= 2, "{keys:?}");
    let pem = std::fs::read_to_string(dir.join("wolos-2026b.pub")).unwrap();
    assert!(keys.contains(&wol_core::package::public_key(&pem).unwrap()));
}
