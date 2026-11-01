// SPDX-License-Identifier: GPL-2.0-or-later
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
            "linux-wolos-api-headers",
            "hyprland-wolos",
            "flatpak-wolos",
            "helium-browser",
            "wolos-mirrorlist",
            "wolos-system",
            "wopkg",
            "wolos-shim",
            "wolos-keyring",
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
        changed("crates/core/src/update.rs\ndesktop/overlay/usr/share/wolos/zsh/zshrc\n"),
        "wolos-system wopkg wolos-base"
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
    // Without one given, the kernel is linux-wolos's full version: kernel/VERSION, epoch, pkgrel.
    let o = Command::new("bash")
        .arg(repo("image/release-manifest.sh"))
        .args([
            iso.to_str().unwrap(),
            "2026-10-20",
            "r42",
            "https://example.org/img",
        ])
        .output()
        .unwrap();
    let m =
        wol_core::update::check::Manifest::parse(&String::from_utf8(o.stdout).unwrap()).unwrap();
    let v = read("kernel/VERSION");
    // The kernel package's release bumps on rebuilds of the same source.
    let pkgrel = read("kernel/linux-wolos/PKGBUILD")
        .lines()
        .find_map(|l| l.strip_prefix("pkgrel=").map(str::to_string))
        .unwrap();
    let get = |k: &str| {
        v.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap()
            .to_string()
    };
    assert_eq!(
        m.kernel,
        format!("1:{}+{}-{}", get("wolkernel"), get("linux"), pkgrel)
    );
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
    let url =
        "https://github.com/wonderlandos/wolos-packages/releases/latest/download/release.json";
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
        "gh release create",
        "--latest",
    ] {
        assert!(iso.contains(want), "iso.yml lacks {want}");
    }
    // Published in the releases repo the check reads (GH_REPO), not in this one.
    assert!(
        iso.contains("GH_REPO: ${{ vars.WOLOS_RELEASES_REPO"),
        "iso.yml GH_REPO"
    );
    assert!(
        !iso.contains("action-gh-release"),
        "iso.yml publishes with the action"
    );
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

/// CI builds from this tree alone: kernel patches included, no second checkout.
#[test]
fn workflows_free_disk_and_need_no_second_checkout() {
    for f in [
        ".github/workflows/packages.yml",
        ".github/workflows/iso.yml",
    ] {
        let s = read(f);
        assert!(s.contains("Free disk space"), "{f}");
        assert!(!s.contains("WOLOS_OVERLAY"), "{f}");
        let low = s.to_lowercase();
        for word in ["private", "proprietary", "closed", "competitor", "strategy"] {
            assert!(!low.contains(word), "{f}: {word}");
        }
    }
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

/// `only: -` signs, indexes and publishes what the release holds (packages built on a build host)
/// without building anything, on the cluster or in the container.
#[test]
fn packages_workflow_signs_prebuilt_packages_without_a_build() {
    let p = read(".github/workflows/packages.yml");
    let step = p.split("      - name: Packages to build").nth(1).unwrap();
    let step = step.split("\n      - name:").next().unwrap();
    let body: String = step
        .split("run: |\n")
        .nth(1)
        .unwrap()
        .lines()
        .map(|l| l.strip_prefix("          ").unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n");
    let t = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("packages-plan");
    let _ = fs::remove_dir_all(&t);
    fs::create_dir_all(t.join("bin")).unwrap();
    let gh = t.join("bin/gh");
    fs::write(&gh, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let src = t.join("src");
    for f in ["image/arch/snapshot", "image/repo/packages"] {
        fs::create_dir_all(src.join(f).parent().unwrap()).unwrap();
        fs::copy(repo(f), src.join(f)).unwrap();
    }
    let out = t.join("out.txt");
    let o = Command::new("bash")
        .arg("-euo")
        .arg("pipefail")
        .arg("-c")
        .arg(&body)
        .current_dir(&src)
        .env(
            "PATH",
            format!(
                "{}:{}",
                t.join("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("ONLY_INPUT", "-")
        .env("GITHUB_OUTPUT", &out)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let got = fs::read_to_string(&out).unwrap();
    assert!(
        got.contains("only=-\n") && got.contains("build=\n"),
        "{got}"
    );
    let o = Command::new("bash")
        .arg("-euo")
        .arg("pipefail")
        .arg("-c")
        .arg(&body)
        .current_dir(&src)
        .env(
            "PATH",
            format!(
                "{}:{}",
                t.join("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("ONLY_INPUT", "cdn")
        .env("GITHUB_OUTPUT", t.join("cdn.txt"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let got = fs::read_to_string(t.join("cdn.txt")).unwrap();
    assert!(
        got.contains("cdn=1\n") && got.contains("only=\n") && got.contains("build=\n"),
        "{got}"
    );
    for step in [
        "      - name: AWS credentials",
        "      - name: Build on the build cluster",
    ] {
        let cond = p.split(step).nth(1).unwrap().lines().nth(1).unwrap();
        assert!(
            cond.contains("steps.plan.outputs.build != ''"),
            "{step}: {cond}"
        );
    }
}

/// A tag whose release already holds an image built on a build host (the cluster had no
/// capacity): no second build; the image must match release.json, which is then signed.
#[test]
fn iso_workflow_signs_a_prebuilt_image_without_building() {
    let iso = read(".github/workflows/iso.yml");
    let step = |name: &str| {
        iso.split(&format!("      - name: {name}"))
            .nth(1)
            .unwrap_or_else(|| panic!("iso.yml: no step {name}"))
            .split("\n      - name:")
            .next()
            .unwrap()
            .to_owned()
    };
    assert!(step("A prebuilt image (tag)").contains("--pattern release.json"));
    for build in [
        "The [wolos-cachy] repo (tag)",
        "mkarchiso in an archlinux container",
        "AWS credentials",
        "mkarchiso on the build cluster",
    ] {
        assert!(
            step(build).contains("steps.pre.outputs.prebuilt != '1'"),
            "{build} runs for a prebuilt image"
        );
    }
    let files = step("Release files");
    assert!(files.contains(r#"m["sha256"]"#) && files.contains(r#"m["size"]"#));
    assert!(files.contains("sha256sum -c"));
    assert!(files.contains("image/sign-release.sh"));
    // Uploaded again only when Secure Boot signing changed the prebuilt image.
    assert!(
        step("Publish the release")
            .contains(r#"if [ "$PREBUILT" != 1 ] || [ "$SB_SIGNED" = 1 ]; then"#)
    );
}

/// Without WOLOS_RELEASES_TOKEN, github.token cannot write to the releases repo (HTTP 403): both
/// workflows keep the signed files as an artifact instead of failing, and the CDN gets the repo
/// only once the release holds it (`only=cdn` pushes the release as it is).
#[test]
fn workflows_keep_signed_files_when_they_cannot_write_the_releases_repo() {
    for f in [
        ".github/workflows/packages.yml",
        ".github/workflows/iso.yml",
    ] {
        let s = read(f);
        assert!(
            s.contains(r#"[ -z "$RELEASES_TOKEN" ] && [ "$GH_REPO" != "$GITHUB_REPOSITORY" ]"#),
            "{f}"
        );
        assert!(s.contains("steps.publish.outputs.published == '0'"), "{f}");
        assert!(s.contains("actions/upload-artifact"), "{f}");
    }
    let p = read(".github/workflows/packages.yml");
    let cdn = p
        .split("      - name: Push the repo to the CDN")
        .nth(1)
        .unwrap();
    let cond = cdn.lines().nth(1).unwrap();
    assert!(
        cond.contains("steps.publish.outputs.published == '1'")
            && cond.contains("steps.plan.outputs.cdn == '1'"),
        "{cond}"
    );
}
