// SPDX-License-Identifier: GPL-2.0-or-later
//! `.wol` double-click wiring in the image (ADR 0011): MIME type, App Store handler, caches.
#![allow(dead_code)] // siblings.rs helpers this file does not call
#[path = "support/siblings.rs"]
mod siblings;

use std::fs;
fn read(p: &str) -> String {
    let path = siblings::root().join(p);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn wol_files_open_in_the_app_store() {
    let mime = read("desktop/overlay/usr/share/mime/packages/wolos-package.xml");
    assert!(mime.contains(&format!(r#"<mime-type type="{}">"#, shared::package::MIME)));
    assert!(mime.contains(r#"<glob pattern="*.wol"/>"#));
    assert!(mime.contains(r#"<alias type="application/x-wol"/>"#));
    let d = read("desktop/overlay/usr/share/applications/org.wolos.store.desktop");
    let line = |k: &str| {
        d.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or_else(|| panic!("{k}= missing"))
            .to_owned()
    };
    assert_eq!(line("Exec"), "wol-store %f");
    assert!(
        line("MimeType")
            .split(';')
            .any(|m| m == shared::package::MIME)
    );
    // The binary ships, and the MIME and desktop caches see the overlay files.
    let rust = read("tools/build-rust.sh");
    let bins = rust
        .lines()
        .find(|l| l.starts_with("for b in wold "))
        .expect("binary list");
    assert!(
        bins.split_whitespace()
            .any(|b| b.trim_end_matches(';') == "wol-store"),
        "{bins}"
    );
    // pacman hooks refresh the desktop database; the MIME database after the package install.
    let inst = read("wolos-system/wolos-system.install");
    assert!(inst.contains("update-mime-database /usr/share/mime"));
}
