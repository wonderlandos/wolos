// SPDX-License-Identifier: GPL-2.0-or-later
//! tools/build-rust.sh with layer-shell present: each cargo call asks only for features of the
//! packages it builds (cargo refuses `-p xguard --features shell/layer-shell`, which stopped the
//! wolos-system build). A stub cargo records the calls and fails the way cargo does.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn stub(dir: &std::path::Path, name: &str, body: &str) {
    let p = dir.join(name);
    fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn cargo_calls_ask_only_for_their_packages_features() {
    let t = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("build-rust");
    let _ = fs::remove_dir_all(&t);
    let bin = t.join("bin");
    let target = t.join("target");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(target.join("release")).unwrap();
    let log = t.join("cargo.log");
    stub(&bin, "pkg-config", "exit 0");
    // Like cargo: `-p P --features Q/f` is an error unless Q is P; a build leaves executables.
    stub(
        &bin,
        "cargo",
        &format!(
            r#"echo "$*" >> {log}
p=; f=; while [ $# -gt 0 ]; do case $1 in -p) p=$2; shift;; --features) f="$f $2"; shift;; esac; shift; done
for x in $f; do case $x in */*) [ -z "$p" ] || [ "${{x%%/*}}" = "$p" ] || {{ echo "error: the package '$p' does not contain this feature: $x" >&2; exit 101; }};; esac; done
for b in wold wol-admind wol wol-login wol-run wol-realtime wol-shell wol-search wol-lock wol-settings wol-loginwindow wol-setup wol-installer wol-window-shot wol-app wol-store wol-monitor wol-flatpak-consent wol-analyze wol-share wol-kit-host wol-plugin-maker wol-webhost wol-xguard wol-xguard-clam pacman wol-authcheck wol-session-pam; do
  printf '#!/bin/sh\nexit 0\n' > {rel}/$b; chmod 755 {rel}/$b; done
for b in wol-quicklook; do printf '#!/bin/sh\nexit 0\n' > {rel}/$b; chmod 755 {rel}/$b; done"#,
            log = log.display(),
            rel = target.join("release").display()
        ),
    );
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let o = Command::new("sh")
        .arg(root().join("tools/build-rust.sh"))
        .arg(t.join("dest"))
        .env("PATH", path)
        .env("CARGO_TARGET_DIR", &target)
        .env_remove("WOLOS_VENDOR")
        .output()
        .unwrap();
    let calls = fs::read_to_string(&log).unwrap_or_default();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!err.contains("does not contain this feature"), "{err}");
    assert!(
        calls.contains("--workspace") && calls.contains("shell/layer-shell"),
        "{calls}"
    );
    assert!(calls.contains("-p xguard-clam"), "{calls}\n{err}");
    // QA-SHADOW-26, ADR 0048: the web-app host is runtime-required (wold takes a call only from
    // /usr/bin/wol-webhost, QA-WEB-4): api webapp.rs HOST; it is installed
    // where HOST names it, from the workspace build (crates/webengine's bin).
    let host = t.join("dest/usr/bin/wol-webhost");
    assert!(
        host.is_file(),
        "build-rust.sh does not install wol-webhost\n{err}"
    );
    let api = fs::read_to_string(root().join("crates/api/src/webapp.rs")).unwrap();
    assert!(api.contains("pub const HOST: &str = \"/usr/bin/wol-webhost\";"));
    // Quick Look: Files starts it through D-Bus, so the binary must land in /usr/bin.
    assert!(
        t.join("dest/usr/bin/wol-quicklook").exists(),
        "wol-quicklook is not installed\n{err}"
    );
}
