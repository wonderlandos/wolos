// SPDX-License-Identifier: GPL-2.0-or-later
//! The module queue scripts against stub commands (audit C8-7): firstboot finishes once the
//! drivers are in, whatever is still queued, and hands the queue to its own unit (collected when
//! it ends, so the next connection can start it again: ST6-2).
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-modscripts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("bin")).unwrap();
    d
}

/// A stub that appends its name and argv to `<dir>/calls` and exits `code`.
fn stub(dir: &Path, name: &str, code: i32) {
    let p = dir.join("bin").join(name);
    let calls = dir.join("calls");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\necho \"{name} $*\" >> '{}'\nexit {code}\n",
            calls.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stub `ip`: with `route` a default route on IPv4 (the machine is online), without none.
/// modules-install counts a failed try only with a route (audit I6-3); the host's `ip` must not
/// decide these tests.
fn ip(dir: &Path, route: bool) {
    let p = dir.join("bin/ip");
    let body = if route {
        "case \"$*\" in *-4*) echo 'default via 10.0.2.2 dev eth0' ;; esac"
    } else {
        "exit 0"
    };
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn firstboot(d: &Path) -> std::process::Output {
    let path = format!(
        "{}:{}",
        d.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    std::process::Command::new("sh")
        .arg(root().join("files/usr/lib/wolos/firstboot"))
        .env("PATH", path)
        .env("WOLOS_STATE", d.join("state"))
        .env_remove("WOL_MODULES_PENDING")
        .env_remove("WOL_PACKAGES_PENDING")
        .output()
        .unwrap()
}

/// C8-7: a queued module that does not install never keeps firstboot-done from being written;
/// the queue goes to the transient drain unit instead. A failed driver step still means "again".
#[test]
fn firstboot_is_done_once_the_drivers_are_whatever_is_queued() {
    for (tag, file) in [("mods", "modules-pending"), ("pkgs", "packages-pending")] {
        let d = temp(tag);
        for c in ["wol", "systemd-run", "logger"] {
            stub(&d, c, 0);
        }
        std::fs::create_dir_all(d.join("state")).unwrap();
        std::fs::write(d.join("state").join(file), "ai-local\n").unwrap();
        let out = firstboot(&d);
        assert!(out.status.success(), "{out:?}");
        assert!(d.join("state/firstboot-done").exists(), "{tag}");
        let calls = std::fs::read_to_string(d.join("calls")).unwrap();
        assert!(
            calls
                .lines()
                .any(|l| l.starts_with("systemd-run ") && l.ends_with("modules-install --drain")),
            "{calls}"
        );
        assert_collected(&calls);
        assert!(
            std::fs::read_to_string(d.join("state").join(file))
                .unwrap()
                .contains("ai-local"),
            "the queue itself is the drain's"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    // Nothing queued: no drain unit. Drivers failed: not done, so the next boot tries again.
    let d = temp("drivers");
    stub(&d, "wol", 1);
    stub(&d, "systemd-run", 0);
    stub(&d, "logger", 0);
    let out = firstboot(&d);
    assert!(out.status.success(), "{out:?}");
    assert!(!d.join("state/firstboot-done").exists());
    let calls = std::fs::read_to_string(d.join("calls")).unwrap();
    assert!(!calls.contains("systemd-run"), "{calls}");
    let _ = std::fs::remove_dir_all(&d);
}

/// ST6-2: the drain's transient unit keeps its one name (`wolos-modules-pending`), so a run that
/// ends `failed` would stay loaded and refuse every later start: `--collect` removes it whatever
/// its end, and exit 1 (something stays queued, the normal end of an offline try) is a success.
fn assert_collected(calls: &str) {
    let l = calls
        .lines()
        .find(|l| l.starts_with("systemd-run "))
        .unwrap_or_else(|| panic!("no systemd-run in {calls}"));
    assert!(l.contains(" --unit=wolos-modules-pending "), "{l}");
    assert!(l.contains(" --collect "), "{l}");
    assert!(l.contains(" --property=SuccessExitStatus=1 "), "{l}");
}

/// The network-up hook starts the same unit the same way, on every connection, and not on `down`.
#[test]
fn the_network_up_hook_starts_a_collected_drain() {
    let d = temp("dispatcher");
    for c in ["systemd-run", "flock", "wol"] {
        stub(&d, c, 0);
    }
    ip(&d, true);
    let path = format!(
        "{}:{}",
        d.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let pending = d.join("modules-pending");
    std::fs::write(&pending, "ai-local\n").unwrap();
    let hook = |action: &str| hook_on(&d, &path, &pending, "eth0", action);
    let calls = hook("up");
    assert_collected(&calls);
    assert!(calls.contains("modules-install --drain"), "{calls}");
    assert!(
        !hook("down").contains("systemd-run"),
        "only a connection coming up drains"
    );
    // The loopback device comes up first on every boot, and a link without a default route
    // cannot download: neither takes the unit's name from the real connection a moment later.
    assert!(
        !hook_on(&d, &path, &pending, "lo", "up").contains("systemd-run"),
        "loopback does not drain"
    );
    ip(&d, false);
    assert!(
        !hook("up").contains("systemd-run"),
        "no default route: no drain"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The hook as NetworkManager runs it: IFACE ACTION, the stubs in `path`; the calls it made.
fn hook_on(d: &Path, path: &str, pending: &Path, iface: &str, action: &str) -> String {
    let _ = std::fs::remove_file(d.join("calls"));
    let out = std::process::Command::new("sh")
        .arg(root().join("desktop/overlay/etc/NetworkManager/dispatcher.d/50-wolos-online"))
        .args([iface, action])
        .env("PATH", path)
        .env("WOL_MODULES_PENDING", pending)
        .env("WOL_PACKAGES_PENDING", d.join("none"))
        .env("WOLOS_LOCK", d.join("lock"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    // The hook's own `flock ... &` is in the background: give it nothing to wait for.
    std::fs::read_to_string(d.join("calls")).unwrap_or_default()
}

/// `modules-install --drain` on `/` with stub pacman/wol/logger and every state file in `d`.
fn drain(d: &Path, extra: &[&str]) -> std::process::Output {
    let path = format!(
        "{}:{}",
        d.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let st = d.join("state");
    std::process::Command::new("sh")
        .arg(root().join("files/usr/lib/wolos/modules-install"))
        .args(extra)
        .env("PATH", path)
        .env("TMPDIR", d)
        .env("WOLOS_PACMAN_DB", d.join("db"))
        .env("WOL_MODULES_DIR", d.join("registry"))
        .env("WOL_MODULES_PENDING", st.join("modules-pending"))
        .env("WOL_PACKAGES_PENDING", st.join("packages-pending"))
        .env("WOL_MODULES_ATTEMPTS", st.join("modules-attempts"))
        .env("WOL_MODULES_GAVE_UP", st.join("modules-gave-up"))
        .env_remove("WOLOS_MODULES_TRIES")
        .output()
        .unwrap()
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// C8-7: a module (and a media package) that can never install is tried at most 5 times, then
/// leaves the queue and is listed as given up; a later install takes it off that list.
#[test]
fn modules_install_gives_up_after_five_failed_drains() {
    let d = temp("giveup");
    ip(&d, true);
    for c in ["wol", "logger", "pacman-conf"] {
        stub(&d, c, 0);
    }
    stub(&d, "pacman", 1);
    let st = d.join("state");
    for sub in ["state", "registry", "db/local"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    std::fs::write(
        d.join("registry/ai-local.json"),
        r#"{"id":"ai-local","name":"Local AI"}"#,
    )
    .unwrap();
    std::fs::write(d.join("registry/dev.json"), r#"{"id":"dev","name":"Dev"}"#).unwrap();
    std::fs::write(st.join("modules-pending"), "ai-local\n").unwrap();
    std::fs::write(st.join("packages-pending"), "ffmpeg\n").unwrap();
    for n in 1..=4 {
        let out = drain(&d, &["--drain"]);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(read(&st.join("modules-pending")), "ai-local\n", "try {n}");
        assert_eq!(read(&st.join("packages-pending")), "ffmpeg\n", "try {n}");
        let tries = read(&st.join("modules-attempts"));
        assert!(tries.contains(&format!("m ai-local {n}\n")), "{tries}");
        assert!(tries.contains(&format!("p ffmpeg {n}\n")), "{tries}");
        assert!(!st.join("modules-gave-up").exists());
    }
    let out = drain(&d, &["--drain"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains(
            "gave up on module ai-local after 5 tries; to try again: wol modules install ai-local"
        ),
        "{err}"
    );
    assert!(
        !st.join("modules-pending").exists(),
        "the id left the queue"
    );
    assert!(!st.join("packages-pending").exists());
    assert!(
        !st.join("modules-attempts").exists(),
        "nothing left to count"
    );
    let gave = read(&st.join("modules-gave-up"));
    assert!(
        gave.contains("m ai-local\n") && gave.contains("p ffmpeg\n"),
        "{gave}"
    );

    // A sixth drain has nothing to do; nothing is downloaded.
    std::fs::remove_file(d.join("calls")).unwrap();
    assert_eq!(drain(&d, &["--drain"]).status.code(), Some(0));
    assert!(!read(&d.join("calls")).contains("pacman "));

    // Installed by hand later: off the gave-up list; another queued id keeps its own count.
    stub(&d, "pacman", 0);
    std::fs::write(st.join("modules-pending"), "dev\n").unwrap();
    std::fs::write(st.join("modules-attempts"), "m dev 2\n").unwrap();
    let out = drain(&d, &["--drain", "ai-local"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(read(&st.join("modules-gave-up")), "p ffmpeg\n");
    assert!(
        !st.join("modules-attempts").exists(),
        "dev installed: forgotten"
    );
    assert!(!st.join("modules-pending").exists());
    let _ = std::fs::remove_dir_all(&d);
}

/// I6-3: a drain without a default route fails like any other, but counts no try: five offline
/// boots must not give a module up before the first connection. Online, the same failure counts.
#[test]
fn offline_drains_use_up_no_tries() {
    let d = temp("offline");
    ip(&d, false);
    for c in ["wol", "logger", "pacman-conf"] {
        stub(&d, c, 0);
    }
    stub(&d, "pacman", 1);
    let st = d.join("state");
    for sub in ["state", "registry", "db/local"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    std::fs::write(
        d.join("registry/ai-local.json"),
        r#"{"id":"ai-local","name":"Local AI"}"#,
    )
    .unwrap();
    std::fs::write(st.join("modules-pending"), "ai-local\n").unwrap();
    std::fs::write(st.join("packages-pending"), "ffmpeg\n").unwrap();
    for n in 1..=7 {
        let out = drain(&d, &["--drain"]);
        assert_eq!(out.status.code(), Some(1), "drain {n}: {out:?}");
        assert_eq!(read(&st.join("modules-pending")), "ai-local\n", "drain {n}");
        assert_eq!(read(&st.join("packages-pending")), "ffmpeg\n", "drain {n}");
        assert!(!st.join("modules-attempts").exists(), "drain {n} counted");
        assert!(!st.join("modules-gave-up").exists(), "drain {n} gave up");
    }
    ip(&d, true);
    let out = drain(&d, &["--drain"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let tries = read(&st.join("modules-attempts"));
    assert!(
        tries.contains("m ai-local 1\n") && tries.contains("p ffmpeg 1\n"),
        "online, the first try: {tries}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// Retry from Settings > Modules (or `modules-install ID` / `--package NAME` by hand): a name asked
/// for again starts its tries from none and leaves the gave-up list while it is queued again.
#[test]
fn asking_for_a_name_again_starts_its_tries_afresh() {
    let d = temp("retry");
    ip(&d, true);
    for c in ["wol", "logger", "pacman-conf"] {
        stub(&d, c, 0);
    }
    stub(&d, "pacman", 1);
    let st = d.join("state");
    for sub in ["state", "registry", "db/local"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    for id in ["ai-local", "dev"] {
        std::fs::write(
            d.join(format!("registry/{id}.json")),
            format!(r#"{{"id":"{id}","name":"{id}"}}"#),
        )
        .unwrap();
    }
    std::fs::write(
        st.join("modules-gave-up"),
        "m ai-local\np ffmpeg\nm console\n",
    )
    .unwrap();
    std::fs::write(st.join("modules-pending"), "dev\n").unwrap();
    std::fs::write(st.join("modules-attempts"), "m dev 3\np lame 2\n").unwrap();
    let out = drain(&d, &["--package", "ffmpeg", "ai-local", "dev"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let tries = read(&st.join("modules-attempts"));
    for line in ["m ai-local 1\n", "m dev 1\n", "p ffmpeg 1\n", "p lame 2\n"] {
        assert!(tries.contains(line), "{line:?} in {tries}");
    }
    assert_eq!(
        read(&st.join("modules-gave-up")),
        "m console\n",
        "asked again: off the list"
    );
    let pending = read(&st.join("modules-pending"));
    assert!(
        pending.contains("ai-local\n") && pending.contains("dev\n"),
        "{pending}"
    );
    assert_eq!(read(&st.join("packages-pending")), "ffmpeg\n");

    // With one try allowed, a failed retry is given up on again at once.
    let out = std::process::Command::new("sh")
        .arg(root().join("files/usr/lib/wolos/modules-install"))
        .arg("ai-local")
        .env(
            "PATH",
            format!(
                "{}:{}",
                d.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("TMPDIR", &d)
        .env("WOLOS_PACMAN_DB", d.join("db"))
        .env("WOL_MODULES_DIR", d.join("registry"))
        .env("WOL_MODULES_PENDING", st.join("modules-pending"))
        .env("WOL_PACKAGES_PENDING", st.join("packages-pending"))
        .env("WOL_MODULES_ATTEMPTS", st.join("modules-attempts"))
        .env("WOL_MODULES_GAVE_UP", st.join("modules-gave-up"))
        .env("WOLOS_MODULES_TRIES", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let gave = read(&st.join("modules-gave-up"));
    assert!(
        gave.contains("m console\n") && gave.contains("m ai-local\n"),
        "{gave}"
    );
    assert!(!read(&st.join("modules-attempts")).contains("ai-local"));
    let _ = std::fs::remove_dir_all(&d);
}
