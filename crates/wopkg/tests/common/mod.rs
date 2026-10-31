//! libwopkg.so.1 for the tests, linked once per test binary by `link.sh` from the staticlib this
//! `cargo test` just built.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Directory holding `libwopkg.so.1` and the `libwopkg.so` link.
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
                n.starts_with("libwopkg-") && n.ends_with(".a")
            })
            .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
            .expect("staticlib built");
        let exe = std::env::current_exe().unwrap();
        let out = std::env::temp_dir().join(format!(
            "wopkg-lib-{}-{}",
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
