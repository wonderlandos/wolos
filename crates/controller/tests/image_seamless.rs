// SPDX-License-Identifier: GPL-2.0-or-later
//! One picture from power-on to the desktop (ADR 0012): loader splash, black console, locked start,
//! compositor colours. Scripts run against fakes; configs are checked line by line.
#[path = "support/repo.rs"]
mod repo;
use repo::{read, repo};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

fn sandbox(tag: &str, fakes: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-seamless-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    for (name, body) in fakes {
        let p = d.join("bin").join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    d
}

fn sh_n(p: &str) {
    let st = Command::new("sh").arg("-n").arg(repo(p)).status().unwrap();
    assert!(st.success(), "sh -n {p}");
}

/// `key = value` lines of one compositor section (`misc {` .. `}`), whitespace trimmed.
fn section(conf: &str, name: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for l in conf.lines().map(str::trim) {
        if l.starts_with(&format!("{name} {{")) {
            inside = true;
        } else if inside && l == "}" {
            inside = false;
        } else if inside && let Some((k, v)) = l.split_once('=') {
            let v = v.split('#').next().unwrap_or("").trim();
            out.push((k.trim().to_string(), v.to_string()));
        }
    }
    out
}

fn value(conf: &str, sect: &str, key: &str) -> Option<String> {
    section(conf, sect)
        .into_iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

#[test]
fn locked_start_hands_hyprland_the_lock_screen() {
    let d = sandbox(
        "session",
        &[(
            "dbus-run-session",
            "#!/bin/sh\necho \"$* | locked=${WOL_SESSION_LOCKED:-} crash=${WOLOS_CRASH_LOCK_CMD:-}\" >> \"$HOME/log\"\n",
        )],
    );
    let home = d.join("home");
    for locked in [true, false] {
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        let mut c = Command::new("sh");
        c.arg(repo("desktop/overlay/usr/bin/wol-session"))
            .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
            .env("HOME", &home)
            .env("XDG_RUNTIME_DIR", &d)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env_remove("WOL_SESSION_LOCKED")
            .env_remove("WOLOS_CRASH_LOCK_CMD");
        if locked {
            c.env("WOL_SESSION_LOCKED", "1");
        }
        assert!(c.status().unwrap().success());
        let log = fs::read_to_string(home.join("log")).unwrap();
        // The variable never reaches apps; a crash restart always relaunches the lock screen.
        let want = if locked {
            "-- start-hyprland -- --locked-cmd wol-lock --startup | locked= crash=wol-lock --startup\n"
        } else {
            "-- start-hyprland | locked= crash=wol-lock --startup\n"
        };
        assert_eq!(log, want, "locked={locked}");
    }
    sh_n("desktop/overlay/usr/bin/wol-session");
}

/// VirtualBox VMSVGA showed no pointer: `cursor:no_hardware_cursors = 2` keeps the cursor plane on
/// every non-NVIDIA GPU, vmwgfx included. In a VM each compositor draws the pointer, and keeps
/// doing so after `hyprctl reload` (`exec`, not `exec-once`); the theme is one that ships.
#[test]
fn every_compositor_draws_the_pointer_in_a_vm_with_a_shipped_theme() {
    let vm = "exec = sh -c 'systemd-detect-virt -q --vm && hyprctl keyword cursor:no_hardware_cursors 1'";
    for f in [
        "hyprland.conf",
        "login.conf",
        "setup.conf",
        "installer.conf",
    ] {
        let c = read(&format!("desktop/overlay/usr/share/wolos/hypr/{f}"));
        assert!(c.lines().any(|l| l.starts_with(vm)), "{f}");
        assert!(c.contains("\nenv = XCURSOR_THEME,Adwaita\n"), "{f}");
        assert!(c.contains("\nenv = XCURSOR_SIZE,24\n"), "{f}");
    }
}

/// Audit4 S5-20: in a VM the desktop draws the pointer as a setting too, so `hyprctl reload` (Settings
/// does it) never brings the invisible cursor plane back. wol-session writes the fragment every
/// start, hyprland.lua loads it after wolos-perf.lua (which sets the auto mode).
#[test]
fn the_desktop_keeps_the_vm_pointer_across_reloads() {
    for (vm, want) in [
        (
            true,
            "-- wol-session: this is a VM\nhl.config({ cursor = { no_hardware_cursors = 1 } })\n",
        ),
        (false, "-- wol-session: not a VM\n"),
    ] {
        let d = sandbox(
            if vm { "vm" } else { "metal" },
            &[
                ("dbus-run-session", "#!/bin/sh\nexit 0\n"),
                (
                    "systemd-detect-virt",
                    if vm {
                        "#!/bin/sh\nexit 0\n"
                    } else {
                        "#!/bin/sh\nexit 1\n"
                    },
                ),
            ],
        );
        let home = d.join("home");
        fs::create_dir_all(&home).unwrap();
        let st = Command::new("sh")
            .arg(repo("desktop/overlay/usr/bin/wol-session"))
            .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
            .env("HOME", &home)
            .env("XDG_RUNTIME_DIR", &d)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env_remove("WOL_SESSION_LOCKED")
            .status()
            .unwrap();
        assert!(st.success());
        let cache = home.join(".cache/wolos");
        let frag = fs::read_to_string(cache.join("hypr-vm.lua")).unwrap();
        assert_eq!(frag, want, "vm={vm}");
        assert!(!cache.join("hypr-vm.lua.tmp").exists());
        // Lua only: no .conf of the compositor's in the cache or the config.
        assert!(!cache.join("hypr-vm.conf").exists());
        assert!(!home.join(".config/hypr/hyprland.conf").exists());
        let user = fs::read_to_string(home.join(".config/hypr/hyprland.lua")).unwrap();
        assert!(
            user.starts_with("-- WolOS defaults. Add your overrides below.\nrequire(\"/usr/share/wolos/hypr/hyprland.lua\")\n"),
            "{user}"
        );
    }
    let desktop = read("desktop/overlay/usr/share/wolos/hypr/hyprland.lua");
    let at = |l: &str| desktop.find(l).unwrap_or_else(|| panic!("{l}"));
    assert!(
        at("\nrequire(\"/usr/share/wolos/hypr/wolos-perf.lua\")\n")
            < at(
                "\nlocal vm_lua = (os.getenv(\"HOME\") or \"\") .. \"/.cache/wolos/hypr-vm.lua\"\n"
            )
    );
    // Loaded only when wol-session wrote it: a start without wol-session shows no error banner.
    assert!(desktop.contains("\nif vm_file then\n    vm_file:close()\n    require(vm_lua)\nend\n"));
}

#[test]
fn every_compositor_starts_on_black_with_the_lock_kept_across_crashes() {
    let desktop = read("desktop/overlay/usr/share/wolos/hypr/hyprland.conf");
    let black = Some("rgb(000000)".to_string());
    assert_eq!(value(&desktop, "misc", "background_color"), black);
    // A crashed wol-lock is replaced by its supervisor; the new one takes the lock over.
    assert_eq!(
        value(&desktop, "misc", "allow_session_lock_restore"),
        Some("true".into())
    );
    for f in ["login.conf", "setup.conf"] {
        let c = read(&format!("desktop/overlay/usr/share/wolos/hypr/{f}"));
        assert_eq!(value(&c, "misc", "background_color"), black, "{f}");
        // No pointer over the black frames: the window shows it after its first frame (kit::Reveal).
        assert_eq!(
            value(&c, "cursor", "invisible"),
            Some("true".into()),
            "{f}: cursor hidden until the window maps"
        );
    }
}

/// The compositor draws no title bars: GTK 3 apps without a header bar (Shotwell) lost their traffic lights.
#[test]
fn every_gtk3_window_draws_the_kit_traffic_lights() {
    let s = read("desktop/overlay/usr/bin/wol-session");
    assert!(s.contains("export GTK_CSD=${GTK_CSD:-1}"), "{s}");
}

#[test]
fn logind_session_runs_on_the_user_bus() {
    // wol-session-pam's session has user@.service: its bus carries PipeWire, the portals and the
    // polkit agent; a private dbus-run-session bus would hide the session from all of them.
    let d = sandbox(
        "userbus",
        &[
            (
                "dbus-run-session",
                "#!/bin/sh\necho private >> \"$HOME/log\"\n",
            ),
            (
                "start-hyprland",
                "#!/bin/sh\necho \"hyprland $DBUS_SESSION_BUS_ADDRESS\" >> \"$HOME/log\"\n",
            ),
        ],
    );
    let (home, rt) = (d.join("home"), d.join("rt"));
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&rt).unwrap();
    let _bus = std::os::unix::net::UnixListener::bind(rt.join("bus")).unwrap();
    let st = Command::new("sh")
        .arg(repo("desktop/overlay/usr/bin/wol-session"))
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("HOME", &home)
        .env("XDG_RUNTIME_DIR", &rt)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("WOL_SESSION_LOCKED")
        .status()
        .unwrap();
    assert!(st.success());
    assert_eq!(
        fs::read_to_string(home.join("log")).unwrap(),
        format!("hyprland unix:path={}/bus\n", rt.display())
    );
}

/// VirtualBox (VMSVGA): Install WolOS opens without kit::Reveal, so a hidden
/// pointer in installer.conf never came back.
#[test]
fn the_installer_compositor_never_hides_the_pointer() {
    let c = read("desktop/overlay/usr/share/wolos/hypr/installer.conf");
    assert_eq!(value(&c, "cursor", "invisible"), None, "installer.conf");
}

/// vmwgfx (VirtualBox VMSVGA, VMware) has a render node, but the compositor drops GTK's GL clients there
/// (wol-shell and every wol-lock died: black screen). Cairo wins over ngl on vmwgfx.
#[test]
fn vmwgfx_sessions_draw_gtk_with_cairo() {
    let s = read("desktop/overlay/usr/bin/wol-session");
    let probe = s
        .find("/sys/class/drm/renderD*/device/driver")
        .expect("no render node driver probe");
    let vmw = s[probe..].find("= vmwgfx ]").expect("no vmwgfx check") + probe;
    let cairo = s[vmw..]
        .find("GSK_RENDERER=${GSK_RENDERER:-cairo}")
        .expect("vmwgfx not cairo")
        + vmw;
    let ngl = s.find("GSK_RENDERER=${GSK_RENDERER:-ngl}").expect("no ngl");
    assert!(cairo < ngl, "the vmwgfx cairo branch must come before ngl");
}

/// A start without logind (no `XDG_RUNTIME_DIR`): when `/run/user/<uid>` cannot be made (a plain
/// user under root's /run/user, here a parent that does not exist) wol-session uses a private
/// folder of its own, 0700, instead of exporting a path that is not there (finding 10).
#[test]
fn wol_session_makes_a_runtime_folder_without_logind() {
    let s = read("desktop/overlay/usr/bin/wol-session");
    let start = s
        .find("if [ -z \"${XDG_RUNTIME_DIR:-}\" ]; then\n")
        .expect("wol-session sets XDG_RUNTIME_DIR");
    let end = start + s[start..].find("\nfi\n").unwrap() + 4;
    let d = sandbox("runtime", &[]);
    let block = s[start..end].replace("/run/user/", &format!("{}/missing/", d.display()));
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("{block}printf '%s' \"$XDG_RUNTIME_DIR\""))
        .env_remove("XDG_RUNTIME_DIR")
        .env("TMPDIR", &d)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let rt = PathBuf::from(String::from_utf8(out.stdout).unwrap());
    let uid = String::from_utf8(Command::new("id").arg("-u").output().unwrap().stdout).unwrap();
    assert_eq!(rt, d.join(format!("wol-runtime-{}", uid.trim())));
    let m = fs::metadata(&rt).unwrap();
    assert!(m.is_dir());
    assert_eq!(m.permissions().mode() & 0o777, 0o700);
    // The next start reuses it; a link planted at that name is refused.
    let again = Command::new("sh")
        .arg("-c")
        .arg(format!("{block}printf '%s' \"$XDG_RUNTIME_DIR\""))
        .env_remove("XDG_RUNTIME_DIR")
        .env("TMPDIR", &d)
        .output()
        .unwrap();
    assert_eq!(PathBuf::from(String::from_utf8(again.stdout).unwrap()), rt);
    fs::remove_dir(&rt).unwrap();
    std::os::unix::fs::symlink(&d, &rt).unwrap();
    let planted = Command::new("sh")
        .arg("-c")
        .arg(format!("{block}printf '%s' \"$XDG_RUNTIME_DIR\""))
        .env_remove("XDG_RUNTIME_DIR")
        .env("TMPDIR", &d)
        .output()
        .unwrap();
    assert!(!planted.status.success(), "{planted:?}");
    let _ = fs::remove_dir_all(&d);
}
