// SPDX-License-Identifier: GPL-2.0-or-later
//! Memory Saver's kill rules alone, for the limits test guest (image/vm/test/limits), where there
//! is no compositor for its engine to follow. wold runs the same `Storm` and `OomLoop` every tick.
use api::memsaver::storm::{OomLoop, Storm};
use api::memsaver::{cgroup, procs};
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
    let mut seen: HashMap<PathBuf, (Storm, OomLoop)> = HashMap::new();
    let mut slice = Storm::default();
    loop {
        for s in scopes(&root) {
            let (storm, oom) = seen.entry(s.clone()).or_default();
            let hot = storm.step(cgroup::refused(&s));
            if oom.step(cgroup::oom_kills(&s)) || hot {
                println!("E storm {}", s.display());
                cgroup::kill(&s);
            }
        }
        let storm = slice.step(cgroup::refused(&root));
        if storm
            && cgroup::tasks(&root) >= procs::USER_TASKS * 95 / 100
            && let Some(big) = scopes(&root).iter().max_by_key(|s| cgroup::tasks(s))
        {
            println!("E slice-full {}", big.display());
            cgroup::kill(big);
        }
        std::thread::sleep(api::memsaver::TICK);
    }
}
