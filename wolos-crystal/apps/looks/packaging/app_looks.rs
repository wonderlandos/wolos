// SPDX-License-Identifier: GPL-2.0-or-later
//! What this file checks (wolos-pkgbuilds): The wolos-app-looks recipe: Looks (org.wolos.looks), the Crystal theme-from-a-picture app, built, checked and installed from wolos-crystal/apps/looks.
//! The recipe builds with the app's own Makefile against the system libwolkit and libwol (the
//! shards and the wol-hue engine are relative requires inside the wolos tree, so nothing is
//! fetched), runs the app's whole `make spec` in check() and installs the program as `wol-looks`
//! with its launcher entry and the user timer of the schedule. Copy to checks/tests/app_looks.rs.
#![allow(dead_code, unused_imports)]
#[path = "support/siblings.rs"]
mod siblings;

use std::fs;
fn read(p: &str) -> String {
    let path = siblings::root().join(p);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{p}: {e}"))
}

const APP: &str = "wolos-crystal/apps/looks";
const RECIPE: &str = "wolos-app-looks/PKGBUILD";

fn words(line: &str) -> Vec<&str> {
    line.split(['(', ')', ' ', '\'', '"'])
        .filter(|w| !w.is_empty())
        .collect()
}

#[test]
fn the_recipe_builds_checks_and_installs_the_app() {
    let p = read(RECIPE);
    for want in [
        "pkgname=wolos-app-looks",
        "cd \"$srcdir/wolos/wolos-crystal/apps/looks\"",
        "make bin/looks WOLKIT_BUILD=/usr/lib RELEASE='--release --no-debug'",
        "make spec WOLKIT_BUILD=/usr/lib WOLKIT_STYLEC=/usr/bin/wolkit-stylec",
        "\"$pkgdir/usr/bin/wol-looks\"",
        "\"$pkgdir/usr/share/applications/org.wolos.looks.desktop\"",
        "\"$pkgdir/usr/lib/systemd/user/wol-looks-schedule.service\"",
        "\"$pkgdir/usr/lib/systemd/user/wol-looks-schedule.timer\"",
        "\"$pkgdir/usr/share/licenses/$pkgname/LICENSE\"",
        "\"$pkgdir/usr/share/licenses/$pkgname/THIRD-PARTY.md\"",
        "pkgver=0.12.0  # the newest CHANGELOG.md entry at _wolos_commit",
    ] {
        assert!(p.contains(want), "{RECIPE} lacks {want}");
    }
    let depends = p.lines().find(|l| l.starts_with("depends=(")).unwrap();
    for d in [
        "libwolkit", "libwol", "gc", "pcre2", "libyaml", "openssl", "gdk-pixbuf2", "wolos-system",
    ] {
        assert!(words(depends).contains(&d), "{depends} lacks {d}");
    }
    let make = p.lines().find(|l| l.starts_with("makedepends=(")).unwrap();
    assert!(words(make).contains(&"crystal"), "{make} lacks crystal");
    let license = p.lines().find(|l| l.starts_with("license=(")).unwrap();
    assert_eq!(words(license), ["license=", "GPL-2.0-or-later", "MIT"], "{license}");
    assert!(!p.contains("shards install") && !p.contains("shards build"), "{RECIPE} runs shards");
}

#[test]
fn looks_is_a_line_of_packages_built_after_the_engine() {
    let line = read("packages")
        .lines()
        .find(|l| l.split_whitespace().next() == Some("wolos-app-looks"))
        .map(str::to_owned)
        .expect("wolos-app-looks is a line of packages");
    let w: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(w[1], "wolos-app-looks", "{line}");
    let deps: Vec<&str> = w[2].split(',').collect();
    for d in ["libwolkit", "libwol", "wolos-system"] {
        assert!(deps.contains(&d), "{line} lacks {d}");
    }
    assert_eq!(&w[3..], ["wolos-app-looks/"], "{line}");
}

#[test]
fn the_app_has_what_the_recipe_installs_and_runs() {
    let mk = read(&format!("{APP}/Makefile"));
    for want in [
        "bin/looks:",
        "spec: spec-core spec-engine",
        "BUILD := $(abspath $(or $(WOLKIT_BUILD),$(SHARD)/build))",
        "STYLEC := $(or $(WOLKIT_STYLEC),",
        "$(RELEASE)",
        "FONTS := $(ROOT)/crates/wolkit/tests/golden/fonts",
    ] {
        assert!(mk.contains(want), "{APP}/Makefile lacks {want}");
    }
    let desktop = read(&format!("{APP}/org.wolos.looks.desktop"));
    for want in [
        "Exec=wol-looks %f",
        "StartupWMClass=org.wolos.looks",
        "MimeType=image/png;image/jpeg;image/webp;",
    ] {
        assert!(desktop.lines().any(|l| l == want), "the desktop entry lacks {want}");
    }
    let main = read(&format!("{APP}/src/main.cr"));
    assert!(main.contains("app_id: Looks::APP_ID") && main.contains("--scheduled"), "{APP}/src/main.cr");
    assert!(read(&format!("{APP}/src/looks/dirs.cr")).contains("APP_ID = \"org.wolos.looks\""));
    let shard = read(&format!("{APP}/shard.yml"));
    assert!(shard.contains("name: looks") && shard.contains("license: GPL-2.0-or-later"));
    assert!(!shard.contains("dependencies:"), "the app takes no shard dependency");
    let svc = read(&format!("{APP}/packaging/wol-looks-schedule.service"));
    assert!(svc.contains("ExecStart=/usr/bin/wol-looks --scheduled") && svc.contains("Type=oneshot"));
    let timer = read(&format!("{APP}/packaging/wol-looks-schedule.timer"));
    assert!(timer.contains("[Timer]") && timer.contains("WantedBy=timers.target"));
    for f in [
        "spec/golden/make.png",
        "spec/golden/make-kit.png",
        "spec/golden/saved.png",
        "spec/golden/history.png",
        "spec/golden/schedule.png",
        "DESIGN.md",
        "tools/styles.sh",
    ] {
        assert!(siblings::root().join(APP).join(f).exists(), "{APP}/{f} is missing");
    }
    // The engine the app links is in the same tree.
    assert!(siblings::root().join("hue/src/hue/palette.cr").exists());
}
