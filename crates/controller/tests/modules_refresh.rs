// SPDX-License-Identifier: GPL-2.0-or-later
//! `modules-install` on the running system refreshes the repo lists before it downloads and again
//! in the install: a system installed offline has none (install-run refreshes only online), so a
//! plain `-S` said "target not found" on every connection until the module was given up (found by
//! the pre-release offline-modules run). The installer's target (`--root`) is not refreshed here.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-modrefresh-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for sub in ["bin", "state", "registry", "db/local"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    std::fs::write(
        d.join("registry/ai-claude.json"),
        r#"{"id":"ai-claude","name":"AI Assistants"}"#,
    )
    .unwrap();
    d
}

/// A stub that appends its name and argv to `<dir>/calls` and runs `body`.
fn stub(dir: &Path, name: &str, body: &str) {
    let p = dir.join("bin").join(name);
    let calls = dir.join("calls");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\necho \"{name} $*\" >> '{}'\n{body}\n",
            calls.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Online (a default route), stub wol/logger/pacman-conf/arch-chroot; `pacman` as given.
fn setup(d: &Path, pacman: &str) {
    stub(
        d,
        "ip",
        "case \"$*\" in *-4*) echo 'default via 10.0.2.2 dev eth0' ;; esac",
    );
    for c in ["wol", "logger", "pacman-conf", "arch-chroot"] {
        stub(d, c, "exit 0");
    }
    stub(d, "pacman", pacman);
}

fn run(d: &Path, args: &[&str]) -> std::process::Output {
    let st = d.join("state");
    std::process::Command::new("sh")
        .arg(root().join("files/usr/lib/wolos/modules-install"))
        .args(args)
        .env(
            "PATH",
            format!(
                "{}:{}",
                d.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
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

fn pacman_calls(d: &Path) -> Vec<String> {
    std::fs::read_to_string(d.join("calls"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("pacman "))
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_queued_module_drains_on_a_system_without_repo_lists() {
    let d = temp("drain");
    // The pacman of a system installed offline: no repo lists until one is refreshed (-y), so
    // any -S or -Sw without it finds nothing. With them it succeeds.
    setup(
        &d,
        "case \" $* \" in *' -Syw '*|*' -Sy '*) exit 0 ;; *) echo 'error: target not found' >&2; exit 1 ;; esac",
    );
    std::fs::write(d.join("state/modules-pending"), "ai-claude\n").unwrap();
    let out = run(&d, &["--drain"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        !d.join("state/modules-pending").exists(),
        "the queue drained"
    );
    let calls = pacman_calls(&d);
    assert_eq!(calls.len(), 2, "{calls:#?}");
    // The download refreshes the lists in the download copy of the database, never the sealed one.
    assert!(
        calls[0].contains(" --dbpath ")
            && calls[0].contains(" -Syw --noconfirm --needed wolos-module-ai-claude"),
        "{calls:#?}"
    );
    assert!(!calls[0].contains(&format!("--dbpath {}", d.join("db").display())));
    // The install (the /usr/local/bin wrapper: a layer) refreshes too, so it finds what was fetched.
    assert!(
        calls[1].starts_with("pacman -Sy --noconfirm --needed wolos-module-ai-claude"),
        "{calls:#?}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_installers_target_is_not_refreshed_here() {
    let d = temp("root");
    setup(&d, "exit 0");
    // arch-chroot runs its command: the calls are pacman's own.
    stub(&d, "arch-chroot", "shift; exec \"$@\"");
    let t = d.join("target");
    std::fs::create_dir_all(t.join("usr/share/wolos/modules")).unwrap();
    std::fs::copy(
        d.join("registry/ai-claude.json"),
        t.join("usr/share/wolos/modules/ai-claude.json"),
    )
    .unwrap();
    let st = t.join("var/lib/wolos");
    let out = std::process::Command::new("sh")
        .arg(root().join("files/usr/lib/wolos/modules-install"))
        .args(["--root", t.to_str().unwrap(), "--", "ai-claude"])
        .env(
            "PATH",
            format!(
                "{}:{}",
                d.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("WOL_MODULES_ATTEMPTS", st.join("modules-attempts"))
        .env("WOL_MODULES_GAVE_UP", st.join("modules-gave-up"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let calls = pacman_calls(&d);
    assert_eq!(
        calls,
        [
            "pacman -Sw --noconfirm --needed wolos-module-ai-claude",
            "pacman -S --noconfirm --needed wolos-module-ai-claude",
        ],
        "install-run refreshed the target when it was online"
    );
    let _ = std::fs::remove_dir_all(&d);
}
