// SPDX-License-Identifier: BSD-2-Clause
//! Vendor consent defaults (`wol_core::consent::DEFAULTS_DIR`): the file a bundled app's package
//! installs holds every grant it lists as a valid default, and Gallery's holds `trash` and `share`
//! for `/usr/bin/wol-gallery` (E2E-GALLERY: without them the first Delete showed a refusal).

use std::path::Path;
use wol_core::consent::{effective, parse, parse_defaults, set_over};

fn shipped(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn every_shipped_default_file_is_all_valid_defaults() {
    let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wolos-crystal/apps");
    let mut n = 0;
    for e in std::fs::read_dir(&apps).expect("wolos-crystal/apps") {
        let dir = e.expect("entry").path();
        for f in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = f.path();
            if !path.to_string_lossy().ends_with(".consent.json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read");
            let all = parse(&text);
            assert!(!all.is_empty(), "{}: no grants", path.display());
            assert_eq!(
                parse_defaults([text.as_str()]),
                all,
                "{}: a grant a package may not default (see consent::DEFAULTABLE) or a duplicate",
                path.display()
            );
            n += 1;
        }
    }
    assert!(n >= 1, "no *.consent.json found");
}

#[test]
fn gallery_can_trash_and_share_on_a_fresh_system_and_the_person_can_say_no() {
    let text = shipped("wolos-crystal/apps/gallery/org.wolos.gallery.consent.json");
    let d = parse_defaults([text.as_str()]);
    let app = "/usr/bin/wol-gallery";
    let held = effective(None, &d);
    for c in ["trash", "share"] {
        assert!(held.contains(&(app.into(), c.into())), "{c}");
    }
    // The capabilities the app asks wold for (shared::portal Need::Grant) are exactly these.
    assert_eq!(held.len(), 2);
    let off = set_over(None, &d, app, "trash", false);
    assert!(!effective(Some(&off), &d).contains(&(app.into(), "trash".into())));
}
