//! wopkg (ADR 0021) ships in the image: built in the `[wolos]` repo from `wopkg/` and
//! `crates/wopkg`, installed by the image, with libwopkg, its header, completions for zsh, bash and
//! fish, and the official recipes with their index.
use std::{fs, path::PathBuf};

fn read(p: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn wopkg_is_a_repo_package_the_image_installs() {
    let t = read("image/repo/packages");
    let line = t
        .lines()
        .find(|l| l.starts_with("wopkg "))
        .expect("wopkg in image/repo/packages");
    let f: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(f[1], "image/repo/wopkg");
    for p in ["image/repo/wopkg/", "wopkg/", "crates/wopkg/"] {
        assert!(f[3..].contains(&p), "a change in {p} rebuilds it: {line}");
    }
    let image = read("image/arch/packages.x86_64");
    assert!(
        image.lines().any(|l| l.trim() == "wopkg"),
        "the image installs wopkg"
    );
}

#[test]
fn the_package_ships_library_header_completions_and_recipes() {
    let p = read("image/repo/wopkg/PKGBUILD");
    for want in [
        "pkgname=wopkg",
        "/usr/bin/wopkg",
        "/usr/lib/libwopkg.so.1",
        "/usr/include/wopkg.h",
        "/usr/share/zsh/site-functions/_wopkg",
        "/usr/share/bash-completion/completions/wopkg",
        "/usr/share/fish/vendor_completions.d/wopkg.fish",
        "/usr/share/wopkg/recipes/",
        "recipe index recipes",
        "crates/wopkg/link.sh",
        "crystal spec",
    ] {
        assert!(p.contains(want), "PKGBUILD lacks {want}");
    }
    // Builds run in a sandbox: bubblewrap is a hard dependency.
    let deps = p.lines().find(|l| l.starts_with("depends=(")).unwrap();
    assert!(deps.contains("bubblewrap"), "{deps}");
    // Every recipe the package installs exists and pins its sources.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wopkg/recipes");
    let mut n = 0;
    for e in fs::read_dir(&dir).unwrap().flatten() {
        let r = e.path().join("recipe.toml");
        let t = fs::read_to_string(&r).unwrap();
        assert!(
            t.contains("sha256 = \"") && t.contains("url = \"https://"),
            "{}",
            r.display()
        );
        n += 1;
    }
    assert!(n >= 3, "bend, hvm, zenc");
}
