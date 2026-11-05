// SPDX-License-Identifier: GPL-2.0-or-later
//! Audit CA-3: an installed system updates from the channel its image was built from. The live
//! medium's /etc/pacman.conf names it (wolos-iso build.sh: `[NAME]` + `Server = URL`);
//! install-run's `install-channel ROOT` puts the mirrorlist back in that section and moves the
//! mirrorlist's release tag to the channel's. The wolos-mirrorlist package's own file is the input.
#[path = "support/siblings.rs"]
mod siblings;

use std::{fs, path::PathBuf, process::Command};

fn repo(p: &str) -> PathBuf {
    siblings::root().join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

const GH: &str = "https://github.com/wonderlandos/wolos-packages/releases/download";
const INC: &str = "Include = /etc/pacman.d/wolos-mirrorlist";

/// The live medium's pacman.conf as build.sh leaves it: the WolOS section names the image's repo.
fn live_conf(section: &str, server: Option<&str>) -> String {
    let conf = read("profile/airootfs/etc/pacman.conf");
    let line = server.map_or_else(|| INC.to_owned(), |s| format!("Server = {s}"));
    assert!(conf.contains(&format!("\n[wolos]\n{INC}\n")), "{conf}");
    conf.replacen(
        &format!("\n[wolos]\n{INC}\n"),
        &format!("\n[{section}]\n{line}\n"),
        1,
    )
}

/// A root with that pacman.conf and the package's mirrorlist; runs install-channel on it.
fn channel(name: &str, conf: &str) -> (bool, String, String, String) {
    let d = std::env::temp_dir().join(format!("wol-channel-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("etc/pacman.d")).unwrap();
    fs::write(d.join("etc/pacman.conf"), conf).unwrap();
    fs::write(
        d.join("etc/pacman.d/wolos-mirrorlist"),
        read("wolos-mirrorlist/wolos-mirrorlist"),
    )
    .unwrap();
    let o = Command::new("sh")
        .arg(repo("files/usr/lib/wolos/install-channel"))
        .arg(&d)
        .output()
        .unwrap();
    let c = fs::read_to_string(d.join("etc/pacman.conf")).unwrap();
    let l = fs::read_to_string(d.join("etc/pacman.d/wolos-mirrorlist")).unwrap();
    let _ = fs::remove_dir_all(&d);
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
        c,
        l,
    )
}

fn servers(list: &str) -> Vec<&str> {
    list.lines()
        .filter_map(|l| l.strip_prefix("Server = "))
        .collect()
}

fn snapshot() -> String {
    read("profile/snapshot").trim().to_owned()
}

#[test]
fn a_testing_image_installs_a_system_on_testing() {
    let url = format!("{GH}/wolos-testing");
    let (ok, err, conf, list) = channel("testing", &live_conf("wolos-testing", Some(&url)));
    assert!(ok, "{err}");
    assert!(
        conf.contains(&format!("\n[wolos-testing]\n{INC}\n")),
        "the section reads the mirrorlist again: {conf}"
    );
    assert!(!conf.contains("\nServer = https://github.com"), "{conf}");
    // The CDN first (QA-A9-IMAGE-05: its folder is named like the tag, `wolos-testing/`), the
    // release assets of the same tag second.
    let cdn = format!("{}/wolos-testing", wol_core::cdn::PRIMARY);
    assert_eq!(servers(&list), [cdn.as_str(), url.as_str()], "{list}");
    assert!(
        !list.contains(&format!("$repo-{}", snapshot())),
        "no stable tag left: {list}"
    );
    assert!(list.starts_with("# Channel wolos-testing:"), "{list}");
    // Run again on its own result (an install retried): nothing more changes.
    let (ok, _, conf2, _) = channel("testing-again", &conf);
    assert!(ok);
    assert_eq!(conf2, conf);
}

#[test]
fn a_stable_image_keeps_the_package_mirrorlist() {
    let url = format!("{GH}/wolos-{}", snapshot());
    let (ok, err, conf, list) = channel("stable", &live_conf("wolos", Some(&url)));
    assert!(ok, "{err}");
    assert_eq!(
        conf,
        read("profile/airootfs/etc/pacman.conf"),
        "back to the image's file"
    );
    assert_eq!(list, read("wolos-mirrorlist/wolos-mirrorlist"), "untouched");
}

#[test]
fn a_local_repo_is_kept_as_a_server_of_its_own() {
    let (ok, err, _, list) = channel(
        "local",
        &live_conf("wolos-testing", Some("file:///srv/repo/wolos-testing/")),
    );
    assert!(ok, "{err}");
    let s = servers(&list);
    assert_eq!(s.last(), Some(&"file:///srv/repo/wolos-testing"), "{list}");
    assert!(
        s.contains(&format!("{GH}/wolos-testing").as_str()),
        "{list}"
    );
}

#[test]
fn a_live_medium_without_a_server_changes_nothing() {
    let conf = read("profile/airootfs/etc/pacman.conf");
    let (ok, err, c, list) = channel("none", &conf);
    assert!(ok, "{err}");
    assert_eq!(c, conf);
    assert_eq!(list, read("wolos-mirrorlist/wolos-mirrorlist"));
}

#[test]
fn a_server_with_shell_or_sed_characters_is_refused() {
    for bad in [
        "https://x/a b",
        "https://x/$(id)",
        "https://x/a\\b",
        "https://x/a;b",
        "https://x/a'b",
    ] {
        let (ok, _, _, list) = channel("bad", &live_conf("wolos-testing", Some(bad)));
        assert!(!ok, "{bad} accepted");
        assert_eq!(
            list,
            read("wolos-mirrorlist/wolos-mirrorlist"),
            "{bad}: mirrorlist changed"
        );
    }
}

#[test]
fn install_run_sets_the_channel_before_anything_goes_online() {
    let run = read("files/usr/lib/wolos/install-run");
    let ch = run
        .find("sh \"$here/install-channel\" \"$T\"")
        .expect("install-run runs install-channel");
    let online = run.find("pacman -Sy").expect("online step");
    let copy = run.find("unsquashfs").expect("copy");
    assert!(
        copy < ch && ch < online,
        "after the copy, before the first sync"
    );
}
