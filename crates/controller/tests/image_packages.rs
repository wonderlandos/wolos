// SPDX-License-Identifier: GPL-2.0-or-later
//! What the image must ship for what the code relies on: `.wol` double-click wiring (ADR 0011), script
//! interpreters, fonts, devices, phones and shares, optional module records. Reads the overlaid
//! sibling root (wolos + wolos-pkgbuilds + wolos-iso, `support/siblings.rs`).
#![allow(dead_code)] // siblings.rs helpers this file does not call
#[path = "support/siblings.rs"]
mod siblings;

use std::fs;
fn read(p: &str) -> String {
    let path = siblings::root().join(p);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// The package names of the two image lists (comments cut).
fn image() -> Vec<String> {
    let mut v = Vec::new();
    for f in ["profile/packages.x86_64", "profile/packages.desktop"] {
        for l in read(f).lines() {
            let l = l.split('#').next().unwrap().trim();
            v.extend(l.split_whitespace().map(String::from));
        }
    }
    v
}
fn ships(p: &str) -> bool {
    image().iter().any(|x| x == p)
}

/// `"packages": [...]` of a registry record (the records are flat, one string per entry).
fn record_packages(id: &str) -> Vec<String> {
    let path = match id {
        "ime" => "files/usr/share/wolos/modules/ime.json",
        "scanner" => "files/usr/share/wolos/modules/scanner.json",
        _ => panic!("{id}: no such record in this test"),
    };
    let t = read(path);
    let m = wol_core::modules::Module::parse(&t).unwrap_or_else(|e| panic!("{id}: {e}"));
    assert_eq!(m.id, id);
    m.packages
}

#[test]
fn wol_files_open_in_the_app_store() {
    let mime = read("desktop/overlay/usr/share/mime/packages/wolos-package.xml");
    assert!(mime.contains(&format!(r#"<mime-type type="{}">"#, shared::package::MIME)));
    assert!(mime.contains(r#"<glob pattern="*.wol"/>"#));
    assert!(mime.contains(r#"<alias type="application/x-wol"/>"#));
    let d = read("desktop/overlay/usr/share/applications/org.wolos.store.desktop");
    let line = |k: &str| {
        d.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or_else(|| panic!("{k}= missing"))
            .to_owned()
    };
    assert_eq!(line("Exec"), "wol-store %f");
    assert!(
        line("MimeType")
            .split(';')
            .any(|m| m == shared::package::MIME)
    );
    // The binary ships, and the MIME and desktop caches see the overlay files.
    let rust = read("tools/build-rust.sh");
    let bins = rust
        .lines()
        .find(|l| l.starts_with("for b in wold "))
        .expect("binary list");
    assert!(
        bins.split_whitespace()
            .any(|b| b.trim_end_matches(';') == "wol-store"),
        "{bins}"
    );
    // pacman hooks refresh the desktop database; the MIME database after the package install.
    let inst = read("wolos-system/wolos-system.install");
    assert!(inst.contains("update-mime-database /usr/share/mime"));
}

/// QA-PWR-1 / QA-PWR-2 (ADR 0041): `wol script` runs .rb with /usr/bin/ruby and .rsh with rsh.
#[test]
fn script_interpreters_ship() {
    let s = read("crates/controller/src/script.rs");
    assert!(s.contains("\"/usr/bin/ruby\""));
    assert!(ships("ruby") && ships("ruby-stdlib"), "ruby");
    // rsh: once script.rs has the rsh kind (wave3/rsh merge), also assert s.contains("rsh").
    assert!(ships("rsh"), "rsh");
    assert!(
        read("packages").lines().any(|l| l.starts_with("rsh ")),
        "rsh is built in [wolos]"
    );
}

/// QA-GFX-4: Energy Mode / `wol power apply` read power-profiles-daemon.
#[test]
fn power_profiles_daemon_ships_and_is_on() {
    assert!(read("crates/core/src/power/policy.rs").contains("\"powerprofilesctl\""));
    assert!(ships("power-profiles-daemon"));
    assert!(
        read("wolos-system/51-wolos-image.preset")
            .lines()
            .any(|l| l == "enable power-profiles-daemon.service")
    );
}

/// QA-GFX-13: Games Doctor wants /dev/ntsync at boot, so the module loads at boot. The file lives
/// in wolos-system (wolos-pkgbuilds), the one home of the image defaults the code relies on.
#[test]
fn ntsync_loads_at_boot() {
    assert!(read("crates/core/src/games/doctor.rs").contains("/dev/ntsync"));
    assert!(
        read("linux-wolos/config.x86_64")
            .lines()
            .any(|l| l == "CONFIG_NTSYNC=m")
    );
    assert!(
        read("wolos-system/ntsync.conf")
            .lines()
            .any(|l| l == "ntsync")
    );
    assert!(read("wolos-system/PKGBUILD").contains("/usr/lib/modules-load.d/ntsync.conf"));
    // One home: not a second copy here that would install over it.
    assert!(
        !siblings::root()
            .join("files/usr/lib/modules-load.d/ntsync.conf")
            .exists()
    );
}

/// QA-GFX-16: pads open through hidraw as the logged-in user. `steam` depends on `steam-devices`
/// (60-steam-input.rules: Sony, Nintendo, 8BitDo, ... with `uaccess`), measured on the 0.12.1 VM; the
/// image must not drop `steam`, and systemd's 70-joystick.rules covers the rest.
#[test]
fn game_devices_udev() {
    assert!(ships("steam"), "steam brings steam-devices");
    assert!(
        read("profile/pacman.conf").contains("[multilib]"),
        "steam is in multilib"
    );
}

/// QA-GFX-17: the QoS classes the Settings and docs describe need scx_wolos running.
#[test]
fn scx_wolos_ships_and_is_on() {
    assert!(read("kernel/scx_wolos/scx_wolos.service").contains("ExecStart=/usr/bin/scx_wolos"));
    assert!(
        read("linux-wolos/config.x86_64")
            .lines()
            .any(|l| l == "CONFIG_SCHED_CLASS_EXT=y")
    );
    assert!(ships("scx-wolos"));
    assert!(
        read("scx-wolos/60-scx-wolos.preset")
            .lines()
            .any(|l| l == "enable scx_wolos.service")
    );
}

/// QA-GFX-17 / QA-PRIV-14: the unit the scx-wolos package installs (kernel/scx_wolos Makefile
/// `install`) is the one in this repo, it is built against the kernel the image runs (one tag
/// for linux-wolos and scx-wolos), and it is a hardened root unit: only the capabilities a
/// sched_ext loader needs (BPF, struct_ops, the boosted-map pin in bpffs).
#[test]
fn scx_wolos_service_is_installed_hardened_and_built_for_the_shipped_kernel() {
    let mk = read("kernel/scx_wolos/Makefile");
    assert!(mk.contains(
        "install -Dm644 scx_wolos.service $(DESTDIR)/usr/lib/systemd/system/scx_wolos.service"
    ));
    let recipe = read("scx-wolos/PKGBUILD");
    let v = read("linux-wolos/VERSION");
    let lx = v.lines().find_map(|l| l.strip_prefix("linux=")).unwrap();
    assert!(
        recipe.lines().any(|l| l == format!("_linux={lx}")),
        "scx-wolos built for {lx}"
    );
    let sum = |r: &str| {
        r.lines()
            .find_map(|l| l.strip_prefix("sha256sums=('"))
            .map(|l| l[..64].to_owned())
    };
    assert_eq!(
        sum(&recipe),
        sum(&read("linux-wolos/PKGBUILD")),
        "the same tag tarball"
    );
    let unit = read("kernel/scx_wolos/scx_wolos.service");
    let service: Vec<&str> = unit
        .split("[Service]")
        .nth(1)
        .unwrap()
        .split("\n[")
        .next()
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    for want in [
        "NoNewPrivileges=yes",
        "CapabilityBoundingSet=CAP_SYS_ADMIN CAP_BPF CAP_PERFMON CAP_SYS_RESOURCE CAP_SYS_NICE",
        "ProtectSystem=full",
        "ProtectHome=yes",
        "PrivateTmp=yes",
        "PrivateNetwork=yes",
        "RestrictAddressFamilies=AF_UNIX",
        "ProtectKernelModules=yes",
        "ProtectKernelLogs=yes",
        "ProtectControlGroups=yes",
        "ProtectHostname=yes",
        "RestrictNamespaces=yes",
        "RestrictSUIDSGID=yes",
        "LockPersonality=yes",
        "MemoryDenyWriteExecute=yes",
        "SystemCallArchitectures=native",
    ] {
        assert!(service.contains(&want), "scx_wolos.service lacks {want}");
    }
    // /sys/fs/bpf/wolos/boosted is pinned at start: /sys stays writable for it.
    assert!(
        !service.iter().any(
            |l| l.starts_with("ProtectKernelTunables") || l.starts_with("ProtectSystem=strict")
        )
    );
    // Start limits are [Unit] keys; in [Service] systemd ignores them.
    assert!(
        !service.iter().any(|l| l.starts_with("StartLimit")),
        "{service:?}"
    );
    let head = unit.split("[Service]").next().unwrap();
    assert!(head.lines().any(|l| l == "StartLimitBurst=5"));
}

/// QA-GFX-2 / QA-GFX-3: first boot installs the NVIDIA driver the catalogue names and loads it.
/// `nvidia-open-dkms` builds the modules for the running WolKernel: the image needs DKMS and the
/// headers of its own kernel; `lib32-nvidia-utils` needs multilib.
#[test]
fn nvidia_first_boot_builds_for_the_running_kernel() {
    let cat = read("crates/core/src/hardware/catalog.rs");
    assert!(
        cat.contains(r#"packages: &["nvidia-open-dkms", "nvidia-utils", "lib32-nvidia-utils"]"#)
    );
    let fb = read("files/usr/lib/wolos/firstboot");
    assert!(fb.contains("nvidia-open-dkms") && fb.contains("wol hw boot --online"));
    assert!(ships("dkms"), "DKMS builds the module at install");
    let base = read("wolos-base/PKGBUILD");
    assert!(
        base.contains("'linux-wolos-headers'"),
        "headers of the image's kernel"
    );
    assert!(read("profile/pacman.conf").contains("[multilib]"));
    // The unit has the time a DKMS build takes.
    let unit = read("files/usr/lib/systemd/system/wolos-firstboot.service");
    assert!(unit.lines().any(|l| l == "TimeoutStartSec=10min"));
}

/// NET2-2 (network-r2.md): NetworkManager's shared mode (`nmcli dev wifi hotspot`, the Wi-Fi
/// hotspot) runs its own dnsmasq for DHCP and DNS; dnsmasq.service itself is never on.
#[test]
fn hotspot_has_dnsmasq_for_shared_mode() {
    assert!(ships("networkmanager"));
    assert!(ships("dnsmasq"), "NetworkManager shared mode needs dnsmasq");
    for p in [
        "files/usr/lib/systemd/system-preset/50-wolos.preset",
        "wolos-system/51-wolos-image.preset",
    ] {
        assert!(
            !read(p).lines().any(|l| l == "enable dnsmasq.service"),
            "{p}"
        );
    }
}

/// QA-ACC-12 / QA-LIVE-11 (ACC B-03): the language picker and the terminal show 日本語 / 中文 / 한국어 /
/// العربية / עברית / emoji. The fontconfig file lives in wolos-system (one home); `fc-match` on the
/// installed system is the journey `qa100-p3-cjk-fonts-ime`.
#[test]
fn cjk_arabic_hebrew_fonts_ship_with_fallbacks() {
    assert!(ships("noto-fonts") && ships("noto-fonts-cjk") && ships("noto-fonts-emoji"));
    let f = read("wolos-system/70-wolos-cjk-arabic.conf");
    for fam in [
        "Noto Sans CJK SC",
        "Noto Sans CJK JP",
        "Noto Sans CJK KR",
        "Noto Sans Arabic",
        "Noto Sans Hebrew",
        "Noto Color Emoji",
    ] {
        assert!(f.contains(fam), "{fam}");
    }
    assert!(read("wolos-system/PKGBUILD").contains("/etc/fonts/conf.d/70-wolos-cjk-arabic.conf"));
    // The overlay's own rules stay what they were (monospace, Inter) and do not repeat the CJK file.
    for o in ["50-wolos-fonts", "60-wolos-inter"] {
        assert!(!read(&format!("desktop/overlay/etc/fonts/conf.d/{o}.conf")).contains("CJK"));
    }
    assert!(
        !siblings::root()
            .join("desktop/overlay/etc/fonts/conf.d/70-wolos-cjk-arabic.conf")
            .exists(),
        "one home: wolos-system"
    );
}

/// QA-NET-4: .local names through avahi + nss-mdns before resolved.
#[test]
fn mdns_names_resolve_through_nss_mdns() {
    assert!(ships("avahi") && ships("nss-mdns"));
    assert!(read("files/usr/lib/systemd/resolved.conf.d/wolos.conf").contains("MulticastDNS=no"));
    assert!(
        read("wolos-system/nsswitch-mdns.sh").contains("mdns_minimal [NOTFOUND=return] resolve")
    );
    assert_eq!(
        read("wolos-system/wolos-system.install")
            .matches("/usr/lib/wolos/nsswitch-mdns")
            .count(),
        2
    );
}

/// QA-BOOT-5: firmware updates (fwupdmgr), metadata download off by default.
#[test]
fn fwupd_ships_without_auto_download() {
    assert!(ships("fwupd"));
    assert!(
        read("wolos-system/51-wolos-image.preset")
            .lines()
            .any(|l| l == "disable fwupd-refresh.timer")
    );
}

/// QA-FILES-6: phones, cameras and shares in Files.
#[test]
fn gvfs_backends() {
    for p in ["gvfs", "gvfs-mtp", "gvfs-gphoto2", "gvfs-smb", "gvfs-afc"] {
        assert!(ships(p), "{p}");
    }
}

/// QA-ADR-21 / QA-MANUAL-15: the plugin schema at SCHEMA_PATH.
#[test]
fn plugin_schema_is_installed() {
    assert!(
        read("crates/shared/src/plugins.rs").contains("\"/usr/share/wolos/plugins.schema.json\"")
    );
    assert!(read("wolos-system/PKGBUILD").contains(
        "docs/plugins/plugin.schema.json\" \"$pkgdir/usr/share/wolos/plugins.schema.json\""
    ));
}

/// The `ime` (fcitx5) and `scanner` (sane) optional modules: a registry record each, the packages of
/// the meta package recipe in wolos-pkgbuilds (same list, same order), the record's own promises
/// (`ime` needs the session to export the toolkit variables).
#[test]
fn ime_and_scanner_modules_have_records_and_recipes() {
    for (id, want) in [
        (
            "ime",
            &[
                "fcitx5",
                "fcitx5-gtk",
                "fcitx5-qt",
                "fcitx5-configtool",
                "fcitx5-mozc",
                "fcitx5-chinese-addons",
                "fcitx5-hangul",
            ][..],
        ),
        (
            "scanner",
            &["sane", "sane-airscan", "ipp-usb", "simple-scan"][..],
        ),
    ] {
        let pk = record_packages(id);
        assert_eq!(pk, want, "{id}");
        let recipe = read(&format!("wolos-module-{id}/PKGBUILD"));
        let depends = recipe
            .split("depends=(")
            .nth(1)
            .and_then(|r| r.split(')').next())
            .unwrap_or_else(|| panic!("{id}: depends="))
            .split_whitespace()
            .map(String::from)
            .collect::<Vec<_>>();
        assert_eq!(depends[0], "wolos-system", "{id}");
        assert_eq!(
            depends[1..],
            pk[..],
            "{id}: recipe depends = record packages"
        );
        assert!(
            wol_core::modules::IDS.contains(&id),
            "{id} is in modules::IDS (the contract tests loop over it)"
        );
    }
    // Neither module is on the ISO: a module is installed when asked for.
    for p in ["fcitx5", "fcitx5-mozc", "sane", "simple-scan"] {
        assert!(!ships(p), "{p} belongs to a module, not to the image");
    }
}

/// The ime module's session half: with fcitx5 installed a login shell exports XMODIFIERS and
/// QT_IM_MODULE (GTK needs nothing on Wayland); without it nothing is set.
#[test]
fn ime_session_exports_only_with_fcitx5() {
    let f = read("desktop/overlay/etc/profile.d/wolos-ime.sh");
    assert!(f.contains("[ -x /usr/bin/fcitx5 ]"));
    assert!(f.contains("XMODIFIERS=\"${XMODIFIERS:-@im=fcitx}\""));
    assert!(f.contains("QT_IM_MODULE=\"${QT_IM_MODULE:-fcitx}\""));
    assert!(
        !f.lines()
            .any(|l| !l.trim_start().starts_with('#') && l.contains("GTK_IM_MODULE")),
        "GTK_IM_MODULE would switch off text-input-v3 on Wayland"
    );
    // The daemon itself: nothing in the session runs xdg autostart, so the compositor starts it, only
    // when the module installed it (the line is A8's, in hyprland.lua).
    let lua = read("desktop/overlay/usr/share/wolos/hypr/hyprland.lua");
    assert!(
        lua.lines()
            .any(|l| l.contains("[ -x /usr/bin/fcitx5 ]") && l.contains("exec fcitx5 -d")),
        "hyprland.lua starts fcitx5 -d when /usr/bin/fcitx5 exists"
    );
}

/// QA-SYS-14: cfg80211 loading must not log "set-wireless-regdom failed with exit code 1". wireless-regdb's
/// helper exits 1 when no domain is configured (the normal state); the /etc rule of the same name runs
/// our wrapper instead, which succeeds then and still passes a real failure through.
#[test]
fn regdom_helper_is_quiet_when_unset_and_loud_when_it_fails() {
    use std::process::Command;
    let rule = read("files/etc/udev/rules.d/85-regulatory.rules");
    assert!(
        rule.lines()
            .any(|l| l.contains("/module/cfg80211")
                && l.contains("RUN+=\"/usr/lib/wolos/set-regdom\"")),
        "{rule}"
    );
    let script = siblings::root().join("files/usr/lib/wolos/set-regdom");
    let dir = std::env::temp_dir().join(format!("regdom-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("helper");
    fs::write(&fake, "#!/bin/sh\necho ran > \"$0.ran\"\nexit 7\n").unwrap();
    let run = |conf: &str| {
        let c = dir.join("conf");
        fs::write(&c, conf).unwrap();
        let _ = fs::remove_file(dir.join("helper.ran"));
        Command::new("sh")
            .arg(&script)
            .env("WOLOS_REGDOM_CONF", &c)
            .env("WOLOS_REGDOM_HELPER", &fake)
            .status()
            .unwrap()
            .code()
    };
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    // Every line commented, as wireless-regdb ships it: nothing to set, no error, helper not run.
    assert_eq!(
        run("#WIRELESS_REGDOM=\"BR\"\n#WIRELESS_REGDOM=\"US\"\n"),
        Some(0)
    );
    assert!(!dir.join("helper.ran").exists(), "helper must not run");
    // A configured domain runs the helper, and its failure is not hidden.
    assert_eq!(run("WIRELESS_REGDOM=\"BR\"\n"), Some(7));
    assert!(dir.join("helper.ran").exists());
    let _ = fs::remove_dir_all(&dir);
}
