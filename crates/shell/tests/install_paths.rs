//! Absolute `/usr/local/share/...` paths in the shell name directories the image installs (image/build.sh
//! or desktop/overlay). Installed-disk setup shot: no wallpaper behind Setup/lock, because they looked in
//! `/usr/local/share/wol/wallpapers` while the image ships `/usr/local/share/wolos/wallpapers`.
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

#[test]
fn share_paths_are_installed_by_the_image() {
    let r = root();
    let build = std::fs::read_to_string(r.join("image/build.sh")).unwrap();
    let mut files = Vec::new();
    rs_files(&r.join("crates/shell/src"), &mut files);
    let mut bad = Vec::new();
    for f in files {
        let s = std::fs::read_to_string(&f).unwrap();
        for lit in s
            .split('"')
            .skip(1)
            .step_by(2)
            .filter(|t| t.starts_with("/usr/local/share/"))
        {
            // Directory the path lives in, two levels under share: /usr/local/share/<a>/<b>.
            let rel: Vec<&str> = lit.trim_start_matches('/').split('/').take(5).collect();
            let dir = rel.join("/");
            let shipped = r.join("desktop/overlay").join(&dir).exists() || build.contains(&dir);
            if !shipped {
                bad.push(lit.to_string());
            }
        }
    }
    bad.sort();
    bad.dedup();
    assert!(bad.is_empty(), "not installed by the image: {bad:?}");
}
