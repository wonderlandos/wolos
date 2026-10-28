//! Package and release pipeline: `image/repo/packages` (build order + local deps), `changed.sh`
//! (changed paths -> packages to rebuild), the release manifest `wol update` reads and its
//! signature, and the CI workflows that build the `[wolos]` repo and the ISO.
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
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

/// `name dir deps paths...` per line, in build order: every local dep is built before its user.
#[test]
fn package_table_is_in_dependency_order() {
    let t = read("image/repo/packages");
    let mut seen: Vec<String> = Vec::new();
    for l in t
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let f: Vec<&str> = l.split_whitespace().collect();
        assert!(f.len() >= 4, "{l}");
        assert!(
            repo(f[1]).join("PKGBUILD").exists(),
            "{}: no PKGBUILD",
            f[1]
        );
        for d in f[2].split(',').filter(|d| *d != "-") {
            assert!(seen.iter().any(|s| s == d), "{} needs {d} first", f[0]);
        }
        seen.push(f[0].into());
    }
    assert_eq!(
        seen,
        [
            "linux-wolos",
            "hyprland-wolos",
            "wolos-system",
            "wolos-base"
        ]
    );
    let b = read("image/repo/build.sh");
    assert!(b.contains("image/repo/packages") || b.contains("$HERE/packages"));
    assert!(
        b.contains("makechrootpkg"),
        "clean chroot when devtools are there"
    );
}

fn changed(paths: &str) -> String {
    let mut c = Command::new("bash")
        .arg(repo("image/repo/changed.sh"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(paths.as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(o.status.success());
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

#[test]
fn changed_paths_rebuild_the_package_and_its_users() {
    assert_eq!(
        changed("image/repo/hyprland-wolos/PKGBUILD\n"),
        "hyprland-wolos wolos-system wolos-base"
    );
    assert_eq!(
        changed("kernel/linux-wolos/config.x86_64\n"),
        "linux-wolos wolos-base"
    );
    assert_eq!(
        changed("crates/core/src/update.rs\ndesktop/overlay/etc/zshrc\n"),
        "wolos-system wolos-base"
    );
    assert_eq!(
        changed("image/hyprland/patches/x.patch\n"),
        "hyprland-wolos wolos-system wolos-base"
    );
    assert_eq!(changed("image/repo/wolos-base/PKGBUILD\n"), "wolos-base");
    assert_eq!(changed("README.md\narchicture/adr/0001.md\n"), "");
}

/// release-manifest.sh writes exactly what `wol update` parses.
#[test]
fn release_manifest_is_what_updates_read() {
    let d = scratch("manifest");
    let iso = d.join("wolos-0.11.2-x86_64.iso");
    fs::write(&iso, "iso bytes").unwrap();
    let o = Command::new("bash")
        .arg(repo("image/release-manifest.sh"))
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
        .arg(repo("image/sign-release.sh"))
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
    let check = read("image/files/usr/lib/wolos/update-check");
    let url = "https://github.com/wonderlandos/wolos/releases/latest/download/release.json";
    assert!(check.contains(url), "update-check URL");
    let iso = read(".github/workflows/iso.yml");
    for want in [
        "tags:",
        "'v*'",
        "workflow_dispatch",
        "--privileged",
        "archlinux",
        "mkarchiso",
        "release-manifest.sh",
        "sign-release.sh",
        "release.json",
        "release.json.sig",
        ".sha256",
        "concurrency:",
        "make_latest",
    ] {
        assert!(iso.contains(want), "iso.yml lacks {want}");
    }
}

#[test]
fn packages_workflow_builds_changed_dirs_in_an_arch_container() {
    let p = read(".github/workflows/packages.yml");
    for want in [
        "branches: [main]",
        "archlinux",
        "image/repo/changed.sh",
        "image/repo/build.sh",
        "WOLOS_REPO_ONLY",
        "actions/cache",
        "concurrency:",
        "repo-add",
    ] {
        assert!(p.contains(want), "packages.yml lacks {want}");
    }
}

/// CI runs without extra patches: the steps that use them skip when $WOLOS_OVERLAY is absent.
#[test]
fn workflows_free_disk_and_build_without_extra_patches() {
    for f in [
        ".github/workflows/packages.yml",
        ".github/workflows/iso.yml",
    ] {
        let s = read(f);
        assert!(s.contains("Free disk space"), "{f}");
        assert!(s.contains("WOLOS_OVERLAY"), "{f}");
        let low = s.to_lowercase();
        for word in ["private", "proprietary", "closed", "competitor", "strategy"] {
            assert!(!low.contains(word), "{f}: {word}");
        }
    }
}
