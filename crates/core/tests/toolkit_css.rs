//! Every theme's GTK 4 and GTK 3 stylesheets parse cleanly in the real toolkits: GTK skips a rule
//! it cannot parse, and a skipped rule is a silent hole in the WolOS look. Parsed with PyGObject
//! (GTK 3 and GTK 4 cannot share a process); no display needed.

use std::path::Path;
use std::process::Command;
use wol_core::theme::{ThemeCatalog, apps};

const PARSE: &str = r#"
import sys, gi
gi.require_version("Gtk", sys.argv[1])
from gi.repository import Gtk
bad = 0
for path in sys.argv[2:]:
    css = open(path).read()
    p = Gtk.CssProvider()
    def err(_p, section, error, path=path):
        global bad
        bad += 1
        loc = section.to_string() if hasattr(section, "to_string") else section.get_start_line()
        print(f"{path}: {loc}: {error.message}")
    p.connect("parsing-error", err)
    try:
        if sys.argv[1] == "4.0":
            p.load_from_string(css)
        else:
            p.load_from_data(css.encode())
    except Exception as e:
        bad += 1
        print(f"{path}: {e}")
sys.exit(1 if bad else 0)
"#;

fn have_gi(version: &str) -> bool {
    Command::new("python3")
        .args([
            "-c",
            &format!("import gi; gi.require_version('Gtk', '{version}')"),
        ])
        .output()
        .is_ok_and(|o| o.status.success())
}

fn parse(version: &str, files: &[std::path::PathBuf]) -> std::process::Output {
    Command::new("python3")
        .args(["-c", PARSE, version])
        .args(files)
        .output()
        .unwrap()
}

/// The harness itself: a rule GTK cannot parse must fail the check, or a green run proves nothing.
#[test]
fn the_parser_rejects_an_unknown_property() {
    let dir = std::env::temp_dir().join(format!("wol-toolkit-css-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.css");
    std::fs::write(&bad, "headerbar { -gtk-bogus: 1px; }\n").unwrap();
    for version in ["4.0", "3.0"] {
        if have_gi(version) {
            assert!(!parse(version, std::slice::from_ref(&bad)).status.success());
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn every_theme_parses_in_gtk3_and_gtk4() {
    let dir = std::env::temp_dir().join(format!("wol-toolkit-css-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cat = ThemeCatalog::builtin();
    let (mut g4, mut g3) = (Vec::new(), Vec::new());
    for s in cat.list() {
        let t = cat.resolve(&s.id).unwrap();
        for f in apps::render(&t, Path::new("/x")) {
            let out = match f.rel {
                "gtk-4.0/gtk.css" => &mut g4,
                apps::GTK3_THEME_CSS => &mut g3,
                _ => continue,
            };
            // Our rules only: the adw-gtk3 import is not installed on dev hosts.
            let text: String = f
                .text
                .lines()
                .filter(|l| !l.starts_with("@import"))
                .map(|l| format!("{l}\n"))
                .collect();
            let p = dir.join(format!("{}-{}", s.id, f.rel.replace('/', "_")));
            std::fs::write(&p, text).unwrap();
            out.push(p);
        }
    }
    assert_eq!(g4.len(), cat.list().len());
    assert_eq!(g3.len(), cat.list().len());
    for (version, files) in [("4.0", &g4), ("3.0", &g3)] {
        if !have_gi(version) {
            eprintln!("SKIP: python3-gi with GTK {version} not installed; CSS not parsed");
            continue;
        }
        let out = parse(version, files);
        assert!(
            out.status.success(),
            "GTK {version}:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
