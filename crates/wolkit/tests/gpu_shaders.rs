// SPDX-License-Identifier: BSD-2-Clause
//! The checked-in SPIR-V (`shaders/*.spv`, embedded by `gpu::pipelines`) must be what
//! `shaders/build.sh` makes from the checked-in GLSL: `shaders/spv.lock` records FNV-1a 64 of
//! each source and output. Editing a shader without rebuilding fails here. Where the same
//! glslang version is installed (the forge), the sources are also recompiled and compared byte
//! for byte.

use std::path::{Path, PathBuf};
use std::process::Command;

use wolkit::text::cache::{FNV_OFFSET, hash_bytes};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

fn fnv(p: &Path) -> String {
    let data = std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    format!("{:016x}", hash_bytes(FNV_OFFSET, &data))
}

struct Lock {
    glslang: String,
    rows: Vec<[String; 4]>,
}

fn lock() -> Lock {
    let text = std::fs::read_to_string(dir().join("spv.lock")).unwrap_or_default();
    let mut l = Lock {
        glslang: String::new(),
        rows: Vec::new(),
    };
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["glslang", v] => l.glslang = (*v).to_string(),
            [a, b, c, d] => l.rows.push([a, b, c, d].map(|s| (*s).to_string())),
            _ => {}
        }
    }
    l
}

#[test]
fn spirv_lock_matches_the_sources_and_outputs() {
    let l = lock();
    assert_eq!(l.rows.len(), 2, "spv.lock lists ui.vert and ui.frag");
    for [src, src_h, spv, spv_h] in &l.rows {
        assert_eq!(
            &fnv(&dir().join(src)),
            src_h,
            "{src} changed since shaders/build.sh ran; rerun it on a box with glslang"
        );
        assert_eq!(
            &fnv(&dir().join(spv)),
            spv_h,
            "{spv} is not the build of {src}"
        );
    }
}

#[test]
fn recompiling_with_the_locked_glslang_gives_the_same_bytes() {
    let l = lock();
    let Ok(out) = Command::new("glslangValidator").arg("--version").output() else {
        println!("note: glslangValidator not installed; byte comparison skipped");
        return;
    };
    let version = String::from_utf8_lossy(&out.stdout);
    let have = version
        .lines()
        .find_map(|v| v.strip_prefix("Glslang Version: "))
        .unwrap_or("")
        .trim()
        .to_string();
    if have != l.glslang {
        println!(
            "note: glslang {have} is not the locked {}; byte comparison skipped",
            l.glslang
        );
        return;
    }
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gpu-shaders");
    std::fs::create_dir_all(&tmp).unwrap_or_default();
    for [src, _, spv, _] in &l.rows {
        let o = tmp.join(spv);
        let st = Command::new("glslangValidator")
            .args(["-V", "--target-env", "vulkan1.0", "-o"])
            .arg(&o)
            .arg(dir().join(src))
            .output();
        assert!(
            st.is_ok_and(|s| s.status.success()),
            "{src} does not compile"
        );
        assert_eq!(
            std::fs::read(&o).unwrap_or_default(),
            std::fs::read(dir().join(spv)).unwrap_or_default(),
            "{spv} differs from a fresh build"
        );
    }
}
