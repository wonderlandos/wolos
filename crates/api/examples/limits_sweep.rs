//! The fork storm rule alone, for the limits test guest (image/vm/test/limits), where there is no
//! compositor for Memory Saver's engine to follow. wold runs the same `Storm` every tick.
use api::memsaver::{cgroup, storm::Storm};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn scopes(root: &Path) -> Vec<PathBuf> {
    let Ok(d) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    d.flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("app-wol-"))
        })
        .collect()
}

fn main() {
    let root = PathBuf::from("/sys/fs/cgroup/user.slice/user-1000.slice");
    let mut seen: HashMap<PathBuf, Storm> = HashMap::new();
    loop {
        for s in scopes(&root) {
            if seen.entry(s.clone()).or_default().step(cgroup::refused(&s)) {
                println!("E storm {}", s.display());
                cgroup::kill(&s);
            }
        }
        std::thread::sleep(api::memsaver::TICK);
    }
}
