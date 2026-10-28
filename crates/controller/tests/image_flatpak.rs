//! flatpak-wolos: Flatpak from the WolOS fork's `wolos` branch at the commit in `ref`, built from a
//! tarball like linux-wolos, replacing Arch's flatpak and listed in the [wolos] repo.
use std::{fs, path::PathBuf, process::Command};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}
fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn recipe_builds_the_fork_at_the_recorded_commit() {
    let r = read("image/repo/flatpak-wolos/ref");
    let commit = r.trim();
    assert!(
        commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit()),
        "ref is a full commit id: {commit:?}"
    );
    let p = read("image/repo/flatpak-wolos/PKGBUILD");
    for want in [
        "pkgname=flatpak-wolos",
        "url='https://github.com/wonderlandos/flatpak'",
        "$url/archive/$_commit.tar.gz",
        "$(<\"$startdir/ref\")",
        "conflicts=(flatpak)",
        "-D system_bubblewrap=bwrap",
        "-D system_dbus_proxy=xdg-dbus-proxy",
    ] {
        assert!(p.contains(want), "PKGBUILD lacks {want}");
    }
    assert!(!p.contains("git+"), "a tarball, not a clone");
    let dir = repo("image/repo/flatpak-wolos");
    let out = Command::new("bash")
        .arg("-c")
        .arg("startdir=$PWD; source ./PKGBUILD >/dev/null; echo \"${provides[*]}|$_commit\"")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = String::from_utf8_lossy(&out.stdout);
    let (provides, pinned) = out.trim().split_once('|').unwrap();
    assert_eq!(pinned, commit);
    // Same epoch and upstream version as Arch's package, so dependants keep resolving.
    assert!(
        provides.split(' ').any(|p| p == "flatpak=1:1.16.6"),
        "{provides}"
    );
}

#[test]
fn the_repo_builds_it() {
    let t = read("image/repo/packages");
    let line = t
        .lines()
        .find(|l| l.starts_with("flatpak-wolos "))
        .expect("flatpak-wolos in image/repo/packages");
    let f: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(f[1], "image/repo/flatpak-wolos");
    assert!(f[3..].contains(&"image/repo/flatpak-wolos/"), "{line}");
}
