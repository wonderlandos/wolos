//! The Arch Linux image (ADR 0016): mkarchiso profile `image/arch`, pinned to one Arch Linux Archive
//! date, `[wolos]` before the Arch repos, linux-wolos only, the `[wolos]` repo build and the release
//! manifest. Scripts run against fakes; nothing here needs root or the network.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
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
    let d = std::env::temp_dir().join(format!("wol-arch-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}
fn bash_n(p: &str) {
    let o = Command::new("bash")
        .arg("-n")
        .arg(repo(p))
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{p}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}
fn snapshot() -> String {
    read("image/arch/snapshot").trim().to_string()
}
/// Fake commands in DIR/bin that log "name args" to DIR/log.
fn fakes(d: &Path, names: &[(&str, &str)]) -> String {
    let bin = d.join("bin");
    fs::create_dir_all(&bin).unwrap();
    for (n, body) in names {
        let p = bin.join(n);
        fs::write(
            &p,
            format!(
                "#!/bin/sh\necho \"{n} $*\" >> {}\n{body}\n",
                d.join("log").display()
            ),
        )
        .unwrap();
        Command::new("chmod").arg("+x").arg(&p).status().unwrap();
    }
    format!("{}:{}", bin.display(), std::env::var("PATH").unwrap())
}
fn packages() -> Vec<String> {
    let mut v = Vec::new();
    for f in ["image/arch/packages.x86_64", "desktop/packages.txt"] {
        for l in read(f).lines() {
            let l = l.split('#').next().unwrap().trim();
            v.extend(l.split_whitespace().map(String::from));
        }
    }
    v
}

#[test]
fn snapshot_is_one_archive_date() {
    let s = snapshot();
    let parts: Vec<&str> = s.split('-').collect();
    assert!(
        parts.len() == 3
            && [4, 2, 2]
                .iter()
                .zip(&parts)
                .all(|(n, p)| p.len() == *n && p.chars().all(|c| c.is_ascii_digit())),
        "snapshot is YYYY-MM-DD: {s:?}"
    );
}

/// pacman reads only the snapshot date; `[wolos]` comes first so linux-wolos and the WolOS
/// packages win over same-named Arch ones. Arch servers inline (nothing rolls in between
/// releases); `[wolos]` and `[wolos-cachy]` read the dated wolos-mirrorlist (ADR 0019).
#[test]
fn pacman_conf_is_pinned_and_wolos_comes_first() {
    let date = snapshot().replace('-', "/");
    for f in [
        "image/arch/pacman.conf",
        "image/arch/airootfs/etc/pacman.conf",
    ] {
        let c = read(f);
        let repos: Vec<&str> = c
            .lines()
            .filter(|l| l.starts_with('[') && *l != "[options]")
            .collect();
        assert_eq!(repos, ["[wolos]", "[core]", "[extra]", "[multilib]"], "{f}");
        let servers: Vec<&str> = c
            .lines()
            .filter_map(|l| l.strip_prefix("Server = "))
            .collect();
        assert_eq!(servers.len(), 3, "{f}");
        for s in &servers {
            assert_eq!(
                *s,
                format!("https://archive.archlinux.org/repos/{date}/$repo/os/$arch"),
                "{f}"
            );
        }
        let inc: Vec<&str> = c.lines().filter(|l| l.contains("Include")).collect();
        assert_eq!(
            inc,
            [
                "Include = /etc/pacman.d/wolos-mirrorlist",
                "#Include = /etc/pacman.d/wolos-mirrorlist"
            ],
            "{f}: only [wolos] and the gated [wolos-cachy]"
        );
        assert!(c.contains("SigLevel = Required"), "{f}: signed packages");
    }
}

#[test]
fn packages_are_arch_names_with_linux_wolos_only() {
    let p = packages();
    for want in [
        "wolos-base",
        "linux-firmware",
        "intel-ucode",
        "amd-ucode",
        "mkinitcpio-archiso",
        "btrfs-progs",
        "dosfstools",
        "plymouth",
        "nftables",
        "hyprland-wolos",
        "mesa",
        "vulkan-intel",
        "vulkan-radeon",
        "lib32-mesa",
        "intel-media-driver",
        "pipewire",
        "wireplumber",
        "networkmanager",
        "bluez",
        "udisks2",
        "steam",
        "firefox",
        "inter-font",
        "adw-gtk-theme",
    ] {
        assert!(p.iter().any(|x| x == want), "missing package {want}");
    }
    // The base parts come only through wolos-base (ADR 0017), never listed one by one.
    for gone in [
        "linux",
        "base",
        "linux-wolos",
        "sudo",
        "coreutils",
        "zsh",
        "pkg",
        "drm-kmod",
        "linux_base-rl9",
        "linux-steam-utils",
        "bsddialog",
        "webcamd",
        "networkmgr",
        "hyprland",
        "pipewire-spa-oss",
        "fusefs-ntfs",
        "nvidia-open",
    ] {
        assert!(!p.iter().any(|x| x == gone), "{gone} is not shipped");
    }
    assert!(
        p.iter().all(|x| x
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_+.@".contains(c))),
        "pacman package names only"
    );
    let mut sorted = p.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), p.len(), "no package listed twice");
}

#[test]
fn profile_boots_linux_wolos_quietly_with_an_install_entry() {
    bash_n("image/arch/profiledef.sh");
    let d = read("image/arch/profiledef.sh");
    for want in [
        "iso_name=\"wolos\"",
        "bootmodes=('uefi.systemd-boot')",
        "pacman_conf=\"pacman.conf\"",
    ] {
        assert!(d.contains(want), "profiledef.sh lacks {want}");
    }
    let live = read("image/arch/efiboot/loader/entries/01-wolos.conf");
    let inst = read("image/arch/efiboot/loader/entries/02-wolos-install.conf");
    for e in [&live, &inst] {
        assert!(e.contains("vmlinuz-linux-wolos") && e.contains("initramfs-linux-wolos.img"));
        assert!(e.contains(" quiet splash ") && e.contains("loglevel=3"));
    }
    assert!(inst.contains("wolos.install=1") && !live.contains("wolos.install"));
    assert!(read("image/arch/efiboot/loader/loader.conf").contains("timeout 0"));
    let preset = read("image/arch/airootfs/etc/mkinitcpio.d/linux-wolos.preset");
    assert!(preset.contains("/boot/vmlinuz-linux-wolos") && preset.contains("archiso"));
}

/// WOLOS_STAGE_ONLY=1: the profile is staged (desktop packages merged, [wolos] read from the local
/// repo for the build, image files, overlay and design in airootfs) and mkarchiso is not run.
#[test]
fn build_stages_the_profile_from_the_local_repo() {
    bash_n("image/build.sh");
    let d = scratch("stage");
    let local = d.join("repo/x86_64");
    fs::create_dir_all(&local).unwrap();
    fs::write(local.join("wolos.db"), "").unwrap();
    let o = Command::new("bash")
        .arg(repo("image/build.sh"))
        .env("WOLOS_WORK", &d)
        .env("WOLOS_REPO_DIR", d.join("repo"))
        .env("WOLOS_STAGE_ONLY", "1")
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let p = d.join("profile");
    let pk = fs::read_to_string(p.join("packages.x86_64")).unwrap();
    assert!(
        pk.contains("wolos-base\n") && pk.contains("steam\n"),
        "{pk}"
    );
    let pc = fs::read_to_string(p.join("pacman.conf")).unwrap();
    assert!(
        pc.contains(&format!(
            "Server = file://{}/$arch",
            d.join("repo").display()
        )),
        "{pc}"
    );
    // pacstrap writes the database where pacman puts it on any host (install-run moves it into
    // the slot): the build's pacman.conf has no DBPath, the installed system's keeps it.
    assert!(!pc.contains("DBPath"), "{pc}");
    // The installed system's pacman.conf keeps the published [wolos] server.
    let ac = fs::read_to_string(p.join("airootfs/etc/pacman.conf")).unwrap();
    assert!(!ac.contains("file://"));
    assert!(ac.contains("\nDBPath = /usr/lib/pacman/\n"));
    // Live-only files; everything else comes from the wolos-system package (a file in both would
    // stop pacstrap: airootfs is copied first).
    for f in [
        "airootfs/etc/wolos/snapshot",
        "airootfs/etc/sysusers.d/wolos-live.conf",
        "airootfs/etc/wolos/login.json",
        "airootfs/etc/sudoers.d/wolos-live",
    ] {
        assert!(p.join(f).exists(), "{f} not staged");
    }
    assert_eq!(
        fs::read_to_string(p.join("airootfs/etc/wolos/snapshot"))
            .unwrap()
            .trim(),
        snapshot()
    );
    // The live dock gets "Install WolOS" (shell.json is a backup file: the live copy wins).
    let shell = fs::read_to_string(p.join("airootfs/etc/wolos/shell.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&shell).unwrap();
    assert_eq!(v["dock"][0]["id"], "org.wolos.install");
    assert!(
        !p.join("airootfs/usr/lib/systemd/system/wold.service")
            .exists()
    );
    assert!(!p.join("airootfs/usr/bin").exists());
    let sudo = fs::read_to_string(p.join("airootfs/etc/sudoers.d/wolos-live")).unwrap();
    assert_eq!(
        sudo.trim(),
        "live ALL=(root) NOPASSWD: /usr/lib/wolos/install-run, /usr/lib/wolos/install-check"
    );
}

#[test]
fn build_runs_mkarchiso_or_a_container() {
    let s = read("image/build.sh");
    for want in [
        "mkarchiso -v -w",
        "podman run --rm --privileged",
        "archlinux:base-devel",
        "image/repo/build.sh",
        "release-manifest.sh",
    ] {
        assert!(s.contains(want), "image/build.sh lacks {want}");
    }
    for gone in ["mdconfig", "bsdinstall", "pkg ", "freebsd"] {
        assert!(
            !s.to_lowercase().contains(gone),
            "image/build.sh still has {gone}"
        );
    }
}

/// The release manifest: snapshot date, repo revision, image URL and its sha256.
#[test]
fn release_manifest_names_snapshot_repo_image_and_hash() {
    bash_n("image/release-manifest.sh");
    let d = scratch("manifest");
    let iso = d.join("wolos-0.11-x86_64.iso");
    fs::write(&iso, "iso bytes").unwrap();
    let o = Command::new("bash")
        .arg(repo("image/release-manifest.sh"))
        .arg(&iso)
        .arg("2026-10-20")
        .arg("r42")
        .arg("https://example.org/img")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["snapshot"], "2026/10/20");
    assert_eq!(v["wolos_repo"], "r42");
    assert_eq!(v["image"], "https://example.org/img/wolos-0.11-x86_64.iso");
    // sha256("iso bytes")
    let sum = Command::new("sha256sum").arg(&iso).output().unwrap();
    let sum = String::from_utf8_lossy(&sum.stdout);
    assert_eq!(v["sha256"], sum.split_whitespace().next().unwrap());
}

/// image/repo/build.sh: linux-wolos, hyprland-wolos, wolos-system, then wolos-base (its manifest
/// needs the others) with makepkg, then a signed `[wolos]` database. Unsigned only when asked.
#[test]
fn repo_build_makes_the_packages_and_a_signed_database() {
    bash_n("image/repo/build.sh");
    for p in [
        "image/repo/wolos-system/PKGBUILD",
        "image/repo/hyprland-wolos/PKGBUILD",
        "image/repo/wolos-base/PKGBUILD",
        "image/repo/base-manifest.sh",
    ] {
        bash_n(p);
    }
    let d = scratch("repo");
    let path = fakes(
        &d,
        &[
            (
                "makepkg",
                // linux-wolos has an epoch: its file name holds ':' and '+'.
                "echo \"built $(basename \"$PWD\")\" >> \"$(dirname \"$0\")/../log\"\nv=1-1; [ \"$(basename \"$PWD\")\" = linux-wolos ] && v='1:1.0+7.3-1'\nbsdtar --zstd -cf \"$PKGDEST/$(basename \"$PWD\")-$v-x86_64.pkg.tar.zst\" -T /dev/null",
            ),
            ("repo-add", ""),
            ("pacman", ""),
            ("gpg", ""),
        ],
    );
    let run = |extra: &[(&str, &str)]| {
        let mut c = Command::new("bash");
        c.arg(repo("image/repo/build.sh"))
            .env("PATH", &path)
            .env("WOLOS_WORK", &d)
            .env_remove("WOLOS_SIGN_KEY")
            .env_remove("WOLOS_UNSIGNED");
        for (k, v) in extra {
            c.env(k, v);
        }
        c.output().unwrap()
    };
    let o = run(&[]);
    assert_eq!(o.status.code(), Some(2), "no key, not asked unsigned");
    assert!(!d.join("log").exists(), "refused before building");
    let o = run(&[("WOLOS_SIGN_KEY", "ABCD1234")]);
    assert!(
        o.status.success(),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let log = fs::read_to_string(d.join("log")).unwrap();
    let mk = log.lines().filter(|l| l.starts_with("makepkg")).count();
    assert_eq!(mk, 8, "{log}");
    let built: Vec<&str> = log
        .lines()
        .filter_map(|l| l.strip_prefix("built "))
        .collect();
    assert_eq!(
        built,
        [
            "linux-wolos",
            "linux-wolos-api-headers",
            "hyprland-wolos",
            "flatpak-wolos",
            "wolos-mirrorlist",
            "wolos-system",
            "wolos-keyring",
            "wolos-base"
        ]
    );
    assert!(
        log.lines()
            .any(|l| l.starts_with("pacman") && l.contains("-Syw --noconfirm --config")),
        "base parts from the snapshot: {log}"
    );
    assert!(d.join("base.manifest").exists());
    let add = log.lines().find(|l| l.starts_with("repo-add")).unwrap();
    assert!(add.contains("--sign --key ABCD1234"), "{add}");
    assert!(add.contains("x86_64/wolos.db.tar.zst"), "{add}");
    for p in [
        "linux-wolos",
        "hyprland-wolos",
        "wolos-system",
        "wolos-base",
    ] {
        assert!(add.contains(p), "{p} not added: {add}");
        let v = if p == "linux-wolos" {
            "1.1.0.7.3-1"
        } else {
            "1-1"
        };
        assert!(
            d.join(format!("repo/x86_64/{p}-{v}-x86_64.pkg.tar.zst"))
                .exists(),
            "{p}-{v}"
        );
    }
    // Release assets cannot hold ':' or '+': indexed under the names the release serves.
    for f in fs::read_dir(d.join("repo/x86_64")).unwrap().flatten() {
        let n = f.file_name().to_string_lossy().into_owned();
        assert!(!n.contains([':', '+']), "{n}");
    }
    let s = read("image/repo/build.sh");
    assert!(s.contains("--sign") && s.contains("gpg --detach-sign"));
}

#[test]
fn hyprland_fork_and_system_package_recipes() {
    let h = read("image/repo/hyprland-wolos/PKGBUILD");
    for want in [
        "pkgname=hyprland-wolos",
        "provides=(hyprland",
        "conflicts=(hyprland",
        "pkgver=0.56.2",
        "../../hyprland/patches/",
        "patch -Np1",
    ] {
        assert!(h.contains(want), "hyprland-wolos lacks {want}");
    }
    // makepkg finds local sources only beside the PKGBUILD: the patches are applied from $startdir.
    let source = h.lines().find(|l| l.starts_with("source=")).unwrap();
    assert!(
        !source.contains("patch"),
        "patches listed in source: {source}"
    );
    assert!(h.contains("\"$startdir\"/../../hyprland/patches/"));
    let w = read("image/repo/wolos-system/PKGBUILD");
    for want in [
        "pkgname=wolos-system",
        "build-rust.sh",
        "desktop/overlay",
        "image/files",
        "build-plugins.sh",
        "hyprland-wolos",
    ] {
        assert!(w.contains(want), "wolos-system lacks {want}");
    }
    for gone in [
        "image/hyprland/build-fork.sh",
        "image/hyprland/port-files",
        "image/fetch-theme.sh",
        "image/iso.sh",
        "image/installerconfig",
        "image/rcconf.sh",
    ] {
        assert!(!repo(gone).exists(), "{gone} still present");
    }
}

/// wolos-base (ADR 0017): kernel + userland at the release version; non-GNU tools stand in for the
/// GNU ones pacman knows (`provides`/`conflicts`), so `base` still resolves.
#[test]
fn wolos_base_is_one_versioned_base() {
    let b = read("image/repo/wolos-base/PKGBUILD");
    for want in [
        "pkgname=wolos-base",
        "CHANGELOG.md",
        "'base'",
        "'linux-wolos'",
        "'linux-wolos-headers'",
        "'linux-wolos-api-headers'",
        "'uutils-coreutils'",
        "'sudo-rs'",
        "'libarchive'",
        "'zsh'",
        "'systemd'",
        "'wolos-system'",
        "provides=('coreutils=9.12' 'sudo')",
        "conflicts=('coreutils' 'sudo')",
        "usr/share/wolos/base.manifest",
        "os-release",
    ] {
        assert!(b.contains(want), "wolos-base lacks {want}");
    }
    for keep in ["glibc", "gcc-libs", "bash"] {
        assert!(!b.contains(&format!("conflicts=('{keep}")), "{keep} stays");
    }
    // makepkg resolves no `../` source: the tree's os-release is read from $startdir.
    assert!(
        !b.lines()
            .any(|l| l.starts_with("source=") && l.contains("../"))
    );
    assert!(b.contains("\"$startdir/../../files/usr/share/wolos/os-release\""));
    let os = read("image/files/usr/share/wolos/os-release");
    assert!(os.contains("ID=wolos") && os.contains("ID_LIKE=arch"));
    assert!(
        os.contains("VERSION_ID=@VERSION@"),
        "filled by the base build"
    );
}

/// base-manifest.sh PKG...: "sha256  /path" for every file of the base packages, sorted; the
/// package metadata files are not system files.
#[test]
fn base_manifest_lists_every_file_with_its_hash() {
    let d = scratch("basemanifest");
    let root = d.join("root");
    fs::create_dir_all(root.join("usr/bin")).unwrap();
    fs::write(root.join("usr/bin/ls"), "ls").unwrap();
    fs::write(root.join(".PKGINFO"), "pkgname = x").unwrap();
    let pkg = d.join("x-1-1-x86_64.pkg.tar.zst");
    let st = Command::new("bsdtar")
        .arg("--zstd")
        .arg("-cf")
        .arg(&pkg)
        .arg("-C")
        .arg(&root)
        .arg(".PKGINFO")
        .arg("usr")
        .status()
        .unwrap();
    assert!(st.success());
    let o = Command::new("bash")
        .arg(repo("image/repo/base-manifest.sh"))
        .arg(&pkg)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout);
    let sum = Command::new("sha256sum")
        .arg(root.join("usr/bin/ls"))
        .output()
        .unwrap();
    let sum = String::from_utf8_lossy(&sum.stdout);
    assert_eq!(
        out,
        format!("{}  /usr/bin/ls\n", sum.split_whitespace().next().unwrap())
    );
    // shadow's newuidmap carries a file capability: restoring it needs root, the builder is not.
    assert!(read("image/repo/base-manifest.sh").contains("--no-xattrs"));
}

/// wolos-system installs next to the Arch base: a file another package owns makes pacman refuse
/// it, unless wolos-system replaces that package.
#[test]
fn wolos_system_files_do_not_collide_with_arch_packages() {
    // Paths Arch packages own (checked against core.files and extra.files of the snapshot).
    for (path, owner) in [
        ("etc/nftables.conf", "nftables"),
        ("etc/xdg/foot/foot.ini", "foot"),
        ("usr/share/zsh/scripts/newuser", "zsh"),
        ("etc/zsh/zshrc", "grml-zsh-config"),
        ("etc/zsh/zprofile", "zsh"),
    ] {
        for tree in ["desktop/overlay", "image/files"] {
            assert!(
                !repo(&format!("{tree}/{path}")).exists(),
                "{path} is {owner}'s"
            );
        }
    }
    let p = read("image/repo/wolos-system/PKGBUILD");
    assert!(
        p.contains("provides=(default-cursors)"),
        "default-cursors owns icons/default"
    );
    assert!(p.contains("conflicts=(default-cursors)"));
    assert!(p.contains("etc/wolos/nftables.conf") && !p.contains(" etc/nftables.conf"));
}
