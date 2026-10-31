//! libwol.so.1 for the tests, linked once per test binary by `link.sh` from the staticlib this
//! `cargo test` just built.
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Directory holding `libwol.so.1` and the `libwol.so` link.
pub fn lib_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        // target/<profile>/deps/<test>-<hash>
        let deps = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let newest = std::fs::read_dir(&deps)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().unwrap().to_string_lossy();
                n.starts_with("libwol-") && n.ends_with(".a")
            })
            .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
            .expect("staticlib built");
        let exe = std::env::current_exe().unwrap();
        let out = std::env::temp_dir().join(format!(
            "wol-abi-lib-{}-{}",
            exe.file_name().unwrap().to_string_lossy(),
            std::process::id()
        ));
        let st = std::process::Command::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("link.sh"))
            .arg(&newest)
            .arg(&out)
            .status()
            .expect("link.sh");
        assert!(st.success(), "link.sh failed");
        out
    })
}

/// A fresh directory for one test.
pub fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-abi-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Compile `tests/c/<name>.c` against `wol.h` and the built `libwol.so.1`.
pub fn build(name: &str, dir: &Path) -> PathBuf {
    let lib = lib_dir();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = dir.join(name);
    let cc = std::process::Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .arg("-I")
        .arg(manifest.join("include"))
        .arg(manifest.join(format!("tests/c/{name}.c")))
        .arg(format!("-L{}", lib.display()))
        .arg("-lwol")
        .arg(format!("-Wl,-rpath,{}", lib.display()))
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cc");
    assert!(
        cc.status.success(),
        "cc: {}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let needed = std::process::Command::new("readelf")
        .env("LC_ALL", "C")
        .args(["-d"])
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&needed.stdout).contains("[libwol.so.1]"),
        "apps record the soname"
    );
    exe
}
pub mod bindgen;
pub mod fakes;
