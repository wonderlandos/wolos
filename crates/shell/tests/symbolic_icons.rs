//! Every symbolic icon name the shell requests ships in the WolOS theme (design/icons/symbolic, installed
//! by design/icons/install.sh): no fallback to whatever system theme the image has. Symbolic SVGs are
//! fill-only: GTK 4.20 (FreeBSD pkg) draws `stroke` icons as solid blobs (FreeBSD shots: black dot for
//! system-shutdown, purple dot for system-search).
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// `"name-symbolic"` string literals in the shell sources.
fn requested() -> Vec<String> {
    let mut files = Vec::new();
    rs_files(&root().join("crates/shell/src"), &mut files);
    let mut names: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let s = std::fs::read_to_string(f).unwrap();
            s.split('"')
                .skip(1)
                .step_by(2)
                .filter(|t| {
                    t.ends_with("-symbolic")
                        && t.bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                })
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

#[test]
fn every_requested_symbolic_icon_ships_in_the_wolos_theme() {
    let names = requested();
    assert!(names.len() > 30, "found only {} names", names.len());
    let dir = root().join("design/icons/symbolic");
    let missing: Vec<_> = names
        .iter()
        .filter(|n| !dir.join(format!("{n}.svg")).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "{} icons not in design/icons/symbolic: {missing:?}",
        missing.len()
    );
}

#[test]
fn symbolic_icons_are_fill_only() {
    let dir = root().join("design/icons/symbolic");
    let bad: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "svg"))
        .filter(|p| std::fs::read_to_string(p).unwrap().contains("stroke"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(bad.is_empty(), "stroked symbolic icons: {bad:?}");
}

/// `"wolos-*"` full-colour icon names the shell requests ship in design/icons/apps (installed to
/// scalable/apps). FreeBSD setup shot: `wolos-mark` was a broken-image placeholder on the Welcome page.
#[test]
fn every_requested_wolos_app_icon_ships() {
    let mut files = Vec::new();
    rs_files(&root().join("crates/shell/src"), &mut files);
    let mut names: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let s = std::fs::read_to_string(f).unwrap();
            s.split('"')
                .skip(1)
                .step_by(2)
                .filter(|t| {
                    t.starts_with("wolos-")
                        && !t.ends_with("-symbolic")
                        && t.bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                })
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .collect();
    names.sort();
    names.dedup();
    let dir = root().join("design/icons/apps");
    let missing: Vec<_> = names
        .iter()
        .filter(|n| !dir.join(format!("{n}.svg")).is_file())
        .collect();
    assert!(missing.is_empty(), "not in design/icons/apps: {missing:?}");
}
