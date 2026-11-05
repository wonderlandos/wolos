// SPDX-License-Identifier: GPL-2.0-or-later
//! Web-app runtime profiles (ADR 0042): wol-session links each flag file to the user's profile
//! through webruntime/link.sh, never over the user's own file, and every shipped file is what
//! wol_core::webruntime generates and what both launcher styles read the same way.
#[path = "support/repo.rs"]
mod repo;
use repo::repo;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use wol_core::webruntime::{self, Gpu, Profile, Role};

const SHARE: &str = "desktop/overlay/usr/share/wolos";

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-webruntime-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn link(cfg: &Path, gpu: &str) {
    let share = repo(SHARE);
    let st = Command::new("sh")
        .arg(share.join("webruntime/link.sh"))
        .arg(cfg)
        .arg(&share)
        .arg(gpu)
        .status()
        .unwrap();
    assert!(st.success());
}

fn target(cfg: &Path, f: &str) -> Option<PathBuf> {
    fs::read_link(cfg.join(f)).ok()
}

fn want(p: Profile, g: Gpu, r: Role) -> PathBuf {
    repo(SHARE).join(webruntime::dir(p, g)).join(r.file())
}

#[test]
fn linker_follows_the_profile_and_leaves_your_files_alone() {
    let cfg = scratch("link").join("config");
    // Fresh account: Balanced, every role.
    link(&cfg, "soft");
    for r in Role::ALL {
        assert_eq!(
            target(&cfg, r.file()),
            Some(want(Profile::Balanced, Gpu::Soft, r)),
            "{r:?}"
        );
    }
    // A profile and a GPU: the links (ours) move.
    webruntime::save_profile(&cfg, Profile::Light).unwrap();
    link(&cfg, "gpu");
    for r in Role::ALL {
        assert_eq!(
            target(&cfg, r.file()),
            Some(want(Profile::Light, Gpu::Hw, r)),
            "{r:?}"
        );
    }
    // 0.12 accounts: a link to the old shared default is ours too.
    fs::remove_file(cfg.join("code-flags.conf")).unwrap();
    std::os::unix::fs::symlink(
        repo(SHARE).join("code-flags.conf"),
        cfg.join("code-flags.conf"),
    )
    .unwrap();
    // Yours: a file, and a link elsewhere.
    fs::remove_file(cfg.join("electron-flags.conf")).unwrap();
    fs::write(cfg.join("electron-flags.conf"), "--mine\n").unwrap();
    let elsewhere = cfg.join("my-browser-flags");
    fs::write(&elsewhere, "--also-mine\n").unwrap();
    fs::remove_file(cfg.join("atlas-flags.conf")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, cfg.join("atlas-flags.conf")).unwrap();
    // Unknown profile word and tier: Balanced, no GPU.
    fs::write(cfg.join(webruntime::PROFILE_FILE), "turbo\n").unwrap();
    link(&cfg, "maybe");
    assert_eq!(
        fs::read_to_string(cfg.join("electron-flags.conf")).unwrap(),
        "--mine\n"
    );
    assert_eq!(target(&cfg, "atlas-flags.conf"), Some(elsewhere));
    assert_eq!(
        target(&cfg, "code-flags.conf"),
        Some(want(Profile::Balanced, Gpu::Soft, Role::Editor))
    );
    fs::remove_dir_all(cfg.parent().unwrap()).ok();
}

/// The real wol-session, run from the repo: the flag files end up linked to the default profile.
#[test]
fn session_start_links_the_runtime_profile() {
    let d = scratch("session");
    fs::create_dir_all(d.join("bin")).unwrap();
    for (name, body) in [
        ("dbus-run-session", "#!/bin/sh\nexit 0\n"),
        ("wol", "#!/bin/sh\necho 'backend: cuda'\n"),
    ] {
        let p = d.join("bin").join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let home = d.join("home");
    fs::create_dir_all(&home).unwrap();
    let st = Command::new("sh")
        .arg(repo("desktop/overlay/usr/bin/wol-session"))
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("HOME", &home)
        .env("XDG_RUNTIME_DIR", &d)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("WOLOS_SHARE", repo(SHARE))
        .env_remove("WOL_SESSION_LOCKED")
        .status()
        .unwrap();
    assert!(st.success());
    // This machine's tier: the session's own rule (render node, not vmwgfx, wol says a backend).
    let render = |e: &fs::DirEntry| e.file_name().to_string_lossy().starts_with("renderD");
    let node = fs::read_dir("/dev/dri").is_ok_and(|r| r.flatten().any(|e| render(&e)));
    let vmwgfx = fs::read_dir("/sys/class/drm").is_ok_and(|r| {
        r.flatten().filter(render).any(|e| {
            fs::read_link(e.path().join("device/driver"))
                .is_ok_and(|l| l.file_name().is_some_and(|n| n == "vmwgfx"))
        })
    });
    let g = if node && !vmwgfx { Gpu::Hw } else { Gpu::Soft };
    for r in Role::ALL {
        assert_eq!(
            target(&home.join(".config"), r.file()),
            Some(want(Profile::Balanced, g, r)),
            "{r:?}"
        );
    }
    fs::remove_dir_all(&d).ok();
}

/// One launcher passes each line as one argument, the other word-splits the whole file: a
/// shipped line with a space would mean two different command lines.
#[test]
fn shipped_files_are_generated_and_read_the_same_by_both_launchers() {
    for p in Profile::ALL {
        for g in Gpu::ALL {
            for r in Role::ALL {
                let f = want(p, g, r);
                let t = fs::read_to_string(&f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
                let pol = wol_core::webpolicy::load(
                    &repo("files/usr/share/wolos/webpolicy"),
                    wol_core::webpolicy::role_id(r),
                )
                .unwrap()
                .unwrap();
                assert_eq!(t, webruntime::flags_file(p, r, g, &pol), "{}", f.display());
                for l in t.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
                    assert!(!l.contains(char::is_whitespace), "{}: {l}", f.display());
                }
            }
        }
    }
}
