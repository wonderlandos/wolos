// SPDX-License-Identifier: GPL-2.0-or-later
//! First sign-in Setup (shell::first_login): `wol-session` leaves the runtime flag while this
//! account's own Setup is not done, and the compositor's start line runs `wol-setup --first-login`
//! before the shell when the flag is there. The script runs against fakes in a scratch home.
#[path = "support/repo.rs"]
mod repo;
use repo::{read, repo};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root =
            std::env::temp_dir().join(format!("wol-first-login-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for d in ["bin", "home", "run"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        // The compositor never starts: the bus wrapper notes it and ends.
        let fake = root.join("bin/dbus-run-session");
        fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        Scratch { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn state(&self) -> PathBuf {
        self.home().join(".local/state/wolos")
    }

    fn flag(&self) -> PathBuf {
        self.root.join("run/wol-first-login")
    }

    /// One sign-in; `live` = the live system's mark exists.
    fn sign_in(&self, live: bool) {
        let mark = self.root.join("wolos-live");
        if live {
            fs::write(&mark, "").unwrap();
        } else {
            let _ = fs::remove_file(&mark);
        }
        let st = Command::new("sh")
            .arg(repo("desktop/overlay/usr/bin/wol-session"))
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .env("HOME", self.home())
            .env("XDG_RUNTIME_DIR", self.root.join("run"))
            .env("XDG_CONFIG_HOME", self.home().join(".config"))
            .env_remove("XDG_STATE_HOME")
            .env("WOLOS_PROFILE", "/nonexistent")
            .env("WOLOS_LIVE_MARK", &mark)
            .env_remove("WOL_SESSION_LOCKED")
            .status()
            .unwrap();
        assert!(st.success());
    }

    fn write(&self, rel: &Path, text: &str) {
        let p = self.home().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
}

// Drop: removes the scratch folder on every exit path.
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A new account (the first administrator or one added later) gets the setup at its first
/// sign-in and at every one after until it is done; then never again.
#[test]
fn a_new_account_gets_its_setup_until_it_is_done() {
    let s = Scratch::new("new");
    s.sign_in(false);
    assert!(s.flag().exists(), "first sign-in: the setup runs");
    assert!(s.state().join("setup-started").exists());
    // The compositor file now exists (wol-session wrote it); a Later or a crash: asked again.
    s.sign_in(false);
    assert!(s.flag().exists(), "not done yet: asked again");
    s.write(Path::new(".local/state/wolos/setup-later"), "3\n");
    s.sign_in(false);
    assert!(
        s.flag().exists(),
        "wol-setup itself counts the Laters (it exits at once after the third)"
    );
    s.write(Path::new(".local/state/wolos/setup-done"), "done 1\n");
    s.sign_in(false);
    assert!(!s.flag().exists(), "done: never again");
}

/// An account that signed in before this release is not asked: it counts as done.
#[test]
fn an_account_from_before_counts_as_done() {
    let s = Scratch::new("older");
    s.write(Path::new(".config/hypr/hyprland.lua"), "-- mine\n");
    s.sign_in(false);
    assert!(!s.flag().exists());
    assert_eq!(
        fs::read_to_string(s.state().join("setup-done")).unwrap(),
        "done before-first-login\n"
    );
}

/// The live system never shows it and leaves no mark.
#[test]
fn never_on_the_live_system() {
    let s = Scratch::new("live");
    s.sign_in(true);
    assert!(!s.flag().exists());
    assert!(!s.state().join("setup-done").exists());
    assert!(!s.state().join("setup-started").exists());
}

/// The start line: the setup first while the flag is there, then the shell whatever it returned:
/// as the user unit (QA-LIVE-21: restarted when killed), bare only where there is no user manager.
#[test]
fn the_compositor_runs_the_setup_before_the_shell() {
    let lua = read("desktop/overlay/usr/share/wolos/hypr/hyprland.lua");
    let lines: Vec<&str> = lua
        .lines()
        .filter(|l| l.starts_with("once(") && l.contains("wol-shell --supervised"))
        .collect();
    assert_eq!(
        lines,
        [
            r#"once([[sh -c '[ -e "$XDG_RUNTIME_DIR/wol-first-login" ] && wol-setup --first-login; dbus-update-activation-environment --systemd --all; systemctl --user start wolos-shell.service || exec wol-shell --supervised 2>>"$XDG_RUNTIME_DIR/wol-shell.log"']])"#
        ]
    );
}
