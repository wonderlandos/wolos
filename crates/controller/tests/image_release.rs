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

/// Audit 8 #4/#5: the two scripts that download as root fetch over https only, every download
/// capped; xguard-update judges the signed bundle.json before it fetches any data file.
#[test]
fn downloading_scripts_are_https_only_and_capped() {
    for p in [
        "files/usr/lib/wolos/update-check",
        "files/usr/lib/wolos/xguard-update",
    ] {
        let t = read(p);
        for want in ["--proto =https", "--proto-redir =https", "--max-filesize"] {
            assert!(t.contains(want), "{p}: {want}");
        }
        // Every curl is called through get(): nothing is fetched without those options.
        let curls = t
            .lines()
            .filter(|l| l.trim_start().starts_with("curl ") || l.contains("| curl"))
            .count();
        assert_eq!(curls, 1, "{p}: one curl call, in get()");
    }
    // The error report only posts, but over https only and bounded too.
    let r = read("files/usr/lib/wolos/report");
    for want in ["--proto =https", "--proto-redir =https", "--max-time"] {
        assert!(r.contains(want), "report: {want}");
    }
    let x = read("files/usr/lib/wolos/xguard-update");
    let at = |s: &str| x.find(s).unwrap_or_else(|| panic!("xguard-update: {s}"));
    assert!(
        at("wol-xguard rules check") < at(r#"get 536870912 "$T/$f""#),
        "the signature is verified before data files are fetched"
    );
    assert!(
        x.contains("while IFS= read -r f"),
        "the names loop is quoted"
    );
    assert!(
        !x.contains("for f in $("),
        "no word splitting of downloaded names"
    );
    assert!(
        x.contains("*.xgdb | *.yxc | NOTICES"),
        "the same names as bundle::fits"
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

/// QA-PRIV-15: the release is built on Linux 7.3.1 (stable; the two use-after-free fixes and the
/// BPF verifier bound), not the 7.3 merge-window snapshot: linux-wolos/VERSION is the one source,
/// and the headers and scx-wolos recipes build from the same tag.
#[test]
fn kernel_version_is_7_3_1() {
    let v = read("linux-wolos/VERSION");
    let key = |k: &str| {
        v.lines()
            .find_map(|l| l.strip_prefix(k))
            .unwrap_or_else(|| panic!("{k}"))
    };
    assert_eq!((key("wolkernel="), key("linux=")), ("1.0", "7.3.1"));
    for p in ["linux-wolos-api-headers/PKGBUILD", "scx-wolos/PKGBUILD"] {
        let r = read(p);
        assert!(r.lines().any(|l| l == "_linux=7.3.1"), "{p}");
        assert!(r.lines().any(|l| l == "_wk=1.0"), "{p}");
    }
}

/// This repo's own checkout (not the sibling overlay): where `git` runs.
fn checkout() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `git -C dir args`: its trimmed stdout and whether it exited 0; None when git cannot run.
fn git(dir: &std::path::Path, args: &[&str]) -> Option<(bool, String)> {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some((
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).trim().to_owned(),
    ))
}

/// qa100 H18-1: the integration branch descends from the last release, so a release cut from it
/// keeps every fix that release shipped (wave3/int lacked all 38 commits of 0.12.1: the /etc
/// overlay of ADR 0044, the security and stability fixes). `tools/release/last-release.txt`
/// records the tag, its commit and the shell commit it pins; both repos must contain them.
/// A tree with no history to ask (a tarball, a shallow clone without the commit) says so and
/// checks the record only; a full history that lacks the commit fails.
#[test]
fn int_descends_from_last_release() {
    let text = read("tools/release/last-release.txt");
    let field = |k: &str| {
        text.lines()
            .filter(|l| !l.starts_with('#'))
            .find_map(|l| l.strip_prefix(k)?.strip_prefix(' '))
            .unwrap_or_else(|| panic!("last-release.txt has no `{k} ` line"))
            .trim()
            .to_owned()
    };
    let (tag, wolos, shell) = (field("tag"), field("wolos"), field("shell"));
    assert!(tag.starts_with('v'), "tag {tag}");
    for c in [&wolos, &shell] {
        assert!(
            c.len() == 40 && c.bytes().all(|b| b.is_ascii_hexdigit()),
            "a full commit id: {c}"
        );
    }
    let root = checkout();
    for (repo, dir, commit) in [
        ("wolos", root.clone(), &wolos),
        ("crates/shell", root.join("crates/shell"), &shell),
    ] {
        let Some((true, top)) = git(&dir, &["rev-parse", "--show-toplevel"]) else {
            eprintln!("note: {repo} is not a git checkout here: the release gate is not checked");
            continue;
        };
        assert!(
            std::path::Path::new(&top)
                .canonicalize()
                .is_ok_and(|t| dir.canonicalize().is_ok_and(|d| d == t)),
            "{repo}: {} is not its own checkout ({top})",
            dir.display()
        );
        let known = git(&dir, &["cat-file", "-e", &format!("{commit}^{{commit}}")])
            .is_some_and(|(ok, _)| ok);
        if !known {
            let shallow = git(&dir, &["rev-parse", "--is-shallow-repository"])
                .is_some_and(|(_, s)| s == "true");
            assert!(
                shallow,
                "{repo}: the last release {commit} is not in this history at all"
            );
            eprintln!("note: {repo} is a shallow clone without {commit}: not checked");
            continue;
        }
        let (descends, _) = git(&dir, &["merge-base", "--is-ancestor", commit, "HEAD"]).unwrap();
        assert!(
            descends,
            "{repo}: HEAD does not descend from the last release {tag} ({commit}); merge the \
             release branch first (qa100 H18-1)"
        );
    }
    // The record agrees with itself where the tag and the commit are known.
    if let Some((true, at)) = git(
        &root,
        &["rev-parse", "-q", "--verify", &format!("{tag}^{{commit}}")],
    ) {
        assert_eq!(
            at, wolos,
            "{tag} names another commit than last-release.txt"
        );
    }
    if let Some((true, pinned)) = git(&root, &["rev-parse", &format!("{wolos}:crates/shell")]) {
        assert_eq!(pinned, shell, "{wolos} pins another shell commit");
    }
}

/// ADR 0044 (0.12.1 FIX-slot): the `/etc` overlay line every slot reads mounts with
/// `index=off,metacopy=off`, so the overlay takes the new slot's `/etc` as its lower after a
/// switch (with `index=on` it refused it: emergency mode after an update). The installer's line
/// (`install-run`) is the same line the slot builder writes (`wol_core::update::ETC_FSTAB`).
#[test]
fn etc_overlay_line_keeps_index_and_metacopy_off() {
    let line = wol_core::update::ETC_FSTAB;
    let opts = line.split_whitespace().nth(3).expect("fstab options field");
    let opts: Vec<&str> = opts.split(',').collect();
    for want in [
        "index=off",
        "metacopy=off",
        "redirect_dir=follow",
        "xino=off",
        "nfs_export=off",
    ] {
        assert!(opts.contains(&want), "ETC_FSTAB lacks {want}: {line}");
    }
    assert!(line.starts_with("overlay /etc overlay "), "{line}");
    let run = read("files/usr/lib/wolos/install-run");
    let written: Vec<&str> = run
        .lines()
        .map(str::trim)
        .filter(|l| l.contains("overlay /etc overlay"))
        .collect();
    assert!(
        !written.is_empty(),
        "install-run writes no /etc overlay line"
    );
    for l in written {
        assert_eq!(
            l,
            format!("echo \"{line}\""),
            "install-run's /etc overlay line is not ETC_FSTAB"
        );
    }
    assert!(
        repo("docs/adr/0044-etc-overlay-across-slot-switches.md").is_file(),
        "ADR 0044"
    );
}

/// What a released 0.12.0 computer accepts as a release manifest, frozen here (the reader of tag
/// v0.12.0, `crates/core/src/update/check.rs` `Manifest::parse` + `verify`): a JSON object with
/// the strings `version` (not empty), `snapshot` (`YYYY/MM/DD`), `wolos_repo`, `kernel`, `image`
/// and `sha256` (64 hex digits); `cachy` may be missing; any other field is ignored (no
/// `deny_unknown_fields`). Its `.sig` is the raw 64-byte Ed25519 signature of the exact bytes by
/// one of the keys it shipped in /usr/share/wolos/keys (wolos-2026.pub, wolos-2026b.pub).
fn v0120_accepts(text: &str) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let field = |k: &str| -> Result<&str, String> {
        v.get(k)
            .and_then(|x| x.as_str())
            .ok_or_else(|| format!("no string field {k:?}"))
    };
    for k in [
        "version",
        "snapshot",
        "wolos_repo",
        "kernel",
        "image",
        "sha256",
    ] {
        field(k)?;
    }
    if let Some(c) = v.get("cachy") {
        c.as_str().ok_or("cachy is not a string")?;
    }
    if field("version")?.is_empty() {
        return Err("empty version".into());
    }
    let snap = field("snapshot")?.as_bytes();
    let date = snap.len() == 10
        && snap[4] == b'/'
        && snap[7] == b'/'
        && snap
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !date {
        return Err(format!(
            "snapshot {:?} is not YYYY/MM/DD",
            field("snapshot")?
        ));
    }
    let sum = field("sha256")?;
    if sum.len() != 64 || !sum.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("sha256 is not 64 hex digits".into());
    }
    Ok(())
}

/// The 0.12.0 client's keys (tag v0.12.0, desktop/overlay/usr/share/wolos/keys).
const V0120_KEYS: [&str; 2] = [
    "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAfFPRaz+BR0I7bauiiNv3zUy/EPvjD564/L86ARoIXB8=\n-----END PUBLIC KEY-----\n",
    "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA+KnWlFbObFNEFJNBxn4KqhweLzdJJL/swRQxGZIgle4=\n-----END PUBLIC KEY-----\n",
];

/// release.json of v0.12.1 as served on 2026-11-04 by all three places update-check reads (the
/// same bytes at cdn.wolos.org/latest/, wolos.squareweb.app/latest/ and the latest GitHub
/// release of wolos-packages), and the hex of its release.json.sig.
const PUBLISHED_0121: &str = concat!(
    r#"{"version":"0.12.1","snapshot":"2026/11/02","wolos_repo":"20261104-5703164","#,
    r#""kernel":"1:1.0+7.3.1-1","#,
    r#""image":"https://github.com/wonderlandos/wolos-packages/releases/download/v0.12.1/wolos-2026.11.04-x86_64.iso","#,
    r#""sha256":"265f628e1fd05ff55748c1f9bfc3e62fa959e169012a458e3e6663a2b5a191e9","size":2133927936}"#,
    "\n"
);
const PUBLISHED_0121_SIG: &str = concat!(
    "c533ff56266d472f055ce5220dfca972e5e02c6b02a2ae7d5f4dbbc4d492ed91",
    "da3610de9af0d2cebe6065afe0848be72eb52c38227987b0bedf3d18b033f605"
);

/// QA-UPG-1 (server side; lead, 0.13: one channel, stable): the release metadata the tooling
/// publishes is found and accepted by the OLD 0.12.0 check too. 0.12.0 reads only
/// `https://github.com/wonderlandos/wolos-packages/releases/latest/download/release.json` (+ `.sig`);
/// the manifest must keep the six fields it requires, stay signed by a key it ships, and the
/// image must keep trusting those keys (a key rotation that drops them cuts 0.12.0 off). The
/// 0.12.0 daily check itself can only get there once its script no longer stops at
/// `checkupdates` (update_scripts.rs check_runs_without_checkupdates_or_fakeroot): that needs a
/// newer wolos-system in the [wolos] repo 0.12.0 reads (docs/modules/security.md, Updates).
#[test]
fn the_0120_client_reads_the_published_release() {
    // 1. The address: the 0.12.0 URL is the last place the current check reads, and the release
    //    tooling publishes the stable release there as the latest (not a pre-release).
    let check = read("files/usr/lib/wolos/update-check");
    assert!(check.lines().any(|l| l
        == "FALLBACK=${WOLOS_RELEASE_FALLBACK-https://github.com/wonderlandos/wolos-packages/releases/latest/download/release.json}"),
        "update-check no longer reads the address 0.12.0 reads");
    // 2. What release-manifest.sh writes (the manifest the publish step only re-points: image,
    //    sha256, size, an optional mirror) is accepted by the 0.12.0 reader and by today's.
    let d = scratch("v0120");
    let iso = d.join("wolos-2026.11.20-x86_64.iso");
    fs::write(&iso, "iso bytes").unwrap();
    let o = Command::new("bash")
        .arg(repo("release/release-manifest.sh"))
        .args([
            iso.to_str().unwrap(),
            "2026-11-20",
            "20261120-abcdef0",
            "https://cdn.wolos.org/v0.13.0",
            "0.13.0",
            "1:1.0+7.3.1-1",
        ])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let written = String::from_utf8(o.stdout).unwrap();
    let mut m: serde_json::Value = serde_json::from_str(&written).unwrap();
    m["mirror"] =
        "https://github.com/wonderlandos/wolos-packages/releases/download/v0.13.0/x.iso".into();
    let published = serde_json::to_string(&m).unwrap();
    for (what, text) in [
        ("release-manifest.sh", written.as_str()),
        ("as published", &published),
    ] {
        v0120_accepts(text)
            .unwrap_or_else(|e| panic!("0.12.0 refuses the manifest {what}: {e}\n{text}"));
        assert!(
            wol_core::update::check::Manifest::parse(text).is_some(),
            "today's reader refuses the manifest {what}"
        );
    }
    // 3. The live v0.12.1 manifest verifies with a key 0.12.0 ships, and those keys are still in
    //    the image (the signer must keep using one of them).
    let old: Vec<[u8; 32]> = V0120_KEYS
        .iter()
        .map(|p| wol_core::package::public_key(p).unwrap())
        .collect();
    v0120_accepts(PUBLISHED_0121).unwrap();
    let sig = wol_core::update::check::unhex(PUBLISHED_0121_SIG).unwrap();
    assert!(
        matches!(
            wol_core::package::verify_index(PUBLISHED_0121.as_bytes(), Some(&sig), &old),
            Ok(wol_core::package::Trust::Signed(_))
        ),
        "the published release.json is not signed by a key 0.12.0 trusts"
    );
    let now = wol_core::package::read_keys(&repo("desktop/overlay/usr/share/wolos/keys"));
    for k in &old {
        assert!(
            now.contains(k),
            "the image dropped a key 0.12.0 verifies releases with"
        );
    }
}
