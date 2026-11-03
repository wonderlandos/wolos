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
    siblings::root().join(p)
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

/// The check fetches from where releases are published: the CDN's `latest/` folder (the release
/// tooling writes it) under cdn.wolos.org, then under the server's own name, then GitHub's latest
/// release when the CDN has none or is down; the feed (`wopkg::FEEDS`) in the same order, both
/// from `wol_core::cdn::LATEST` (tests/cdn_order.rs runs the order).
#[test]
fn update_check_reads_the_published_release() {
    let check = read("files/usr/lib/wolos/update-check");
    for (var, url) in [
        (
            "URL=${WOLOS_RELEASE_URL:-",
            "https://cdn.wolos.org/latest/release.json}",
        ),
        (
            "MIRROR=${WOLOS_RELEASE_MIRROR-",
            "https://wolos.squareweb.app/latest/release.json}",
        ),
        (
            "FALLBACK=${WOLOS_RELEASE_FALLBACK-",
            "https://github.com/wonderlandos/wolos-packages/releases/latest/download/release.json}",
        ),
    ] {
        assert!(
            check.lines().any(|l| l == format!("{var}{url}")),
            "update-check {var}"
        );
    }
    assert!(
        check.contains("if release \"$URL\" || release \"$MIRROR\" || release \"$FALLBACK\"; then")
    );
    // The manifest and the feeds share one folder, named in one place.
    assert!(
        read("crates/wopkg/src/lib.rs")
            .contains("pub const FEEDS: &str = wol_core::cdn::LATEST[0];"),
        "wopkg::FEEDS"
    );
    // wolos-iso release.yml is reusable: only wolos publish.yml (what=iso, the repo holding the
    // keys) calls it, one publish at a time. No tag push or dispatch of its own.
    let iso = read(".github/workflows/release.yml");
    assert!(iso.contains("workflow_call"), "release.yml is not reusable");
    assert!(
        !iso.contains("workflow_dispatch") && !iso.contains("tags:"),
        "release.yml has a trigger of its own"
    );
    let publish = read(".github/workflows/publish.yml");
    for want in [
        "uses: wonderlandos/wolos-iso/.github/workflows/release.yml@",
        // Audit A3: the keys go to the signing jobs by name, behind the `release` environment.
        "environment: release",
        "WOLOS_RELEASE_KEY: ${{ secrets.WOLOS_RELEASE_KEY }}",
        "concurrency:",
        "iso-s3",
    ] {
        assert!(publish.contains(want), "publish.yml lacks {want}");
    }
    assert!(
        !publish.contains("secrets: inherit"),
        "publish.yml hands every secret to the called workflows"
    );
    // what=iso-s3 publishes a pre-release in wolos: never the latest release update-check reads.
    assert!(
        publish.contains("--prerelease"),
        "iso-s3 is not a pre-release"
    );
    // The ISO is built (build.sh: mkarchiso, then release/release-manifest.sh writes the manifest
    // `wol update` parses next to it) and VM-tested on the forge; release.yml takes it with its
    // checks.txt and hands it to `wolos-build release --do --iso` (wolos-build lib/publish.sh), the
    // one writer: release.json = that manifest with image, sha256 and size of the published ISO,
    // signed with release/sign-release.sh, then the latest release.
    let build = read("build.sh");
    for want in ["mkarchiso", "release/release-manifest.sh"] {
        assert!(build.contains(want), "build.sh lacks {want}");
    }
    for want in [
        "checks.txt",
        "BOOT PASS",
        "wolos-build release --do --iso",
        "sign-release.sh",
        "release.json",
        "release.json.sig",
        ".sha256",
    ] {
        assert!(iso.contains(want), "release.yml lacks {want}");
    }
    assert!(
        !iso.contains("gh release create") && !iso.contains("openssl pkeyutl"),
        "release.yml writes the release itself instead of wolos-build release"
    );
    // The tool, when its private repo is checked out next to this one (the forge; not wolos CI).
    let own = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|d| d.join("LICENSE").is_file())
        .map(|d| d.to_path_buf());
    let tool = own.and_then(|d| d.parent().map(|p| p.join("wolos-build/lib/publish.sh")));
    if let Some(text) = tool.and_then(|p| fs::read_to_string(p).ok()) {
        for want in [
            "release/sign-release.sh",
            // the manifest's own fields stay; only these three follow the published ISO
            "m[\"image\"], m[\"sha256\"], m[\"size\"] = url, sha, os.path.getsize(iso)",
            // a stable release is a draft until every asset is on it (I7-6) ...
            "gh_w release create \"$tag\" --draft --latest=false",
            // ... and becomes the latest (what `wol update` reads) only then
            "gh_w release edit \"$tag\" --draft=false --latest",
            "gh_w release create \"$tag\" --prerelease",
            // and the signed channel feed goes up with it
            "release_feed stable",
        ] {
            assert!(
                text.contains(want),
                "wolos-build lib/publish.sh lacks {want}"
            );
        }
        let upload = text.find("gh_w release upload \"$tag\"");
        let latest = text.find("gh_w release edit \"$tag\" --draft=false --latest");
        assert!(
            matches!((upload, latest), (Some(u), Some(l)) if u < l),
            "lib/publish.sh moves releases/latest before the assets are uploaded"
        );
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

/// release.json as published (wolos v0.11.4, what=iso-s3): pretty-printed, with `size` and an
/// image URL in wonderlandos/wolos releases. `wol update` reads it.
#[test]
fn published_release_json_parses() {
    let text = r#"{
  "version": "0.11.4",
  "snapshot": "2026/10/31",
  "wolos_repo": "20261101-af08675",
  "cachy": "",
  "kernel": "1:1.0+7.3-5",
  "image": "https://github.com/wonderlandos/wolos/releases/download/v0.11.4/wolos-2026.11.01-x86_64.iso",
  "sha256": "1f4fe65c7c196484d30c3e34b5b79a6ba468664714416988be2635826b5b7cb5",
  "size": 2106126336
}
"#;
    let m = wol_core::update::check::Manifest::parse(text).expect("v0.11.4 release.json");
    assert_eq!(m.version, "0.11.4");
    assert!(m.image.ends_with("/v0.11.4/wolos-2026.11.01-x86_64.iso"));
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
