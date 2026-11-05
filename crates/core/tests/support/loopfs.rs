// SPDX-License-Identifier: BSD-2-Clause
//! A loop-mounted btrfs image for the tests that need a file system with fs-verity (tmpfs and
//! most /tmp have none). Needs `sudo -n` (mkfs, mount, umount), `mkfs.btrfs` and a kernel with
//! `CONFIG_FS_VERITY` and btrfs verity; `LoopFs::new` answers None when any is missing and the
//! test says it was skipped. `WOL_REQUIRE_LOOP=1` makes a missing piece a failure (CI boxes that
//! have root must not skip silently).
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct LoopFs {
    dir: PathBuf,
    mnt: PathBuf,
}

fn sudo(args: &[&str]) -> bool {
    Command::new("sudo")
        .arg("-n")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[allow(dead_code)] // the controller's tests include this file and use only `new`, not `sized`
impl LoopFs {
    /// A fresh 512 MiB btrfs mounted under the temp dir and owned by the caller.
    pub fn new(tag: &str) -> Option<LoopFs> {
        Self::sized(tag, 512)
    }
    /// [`Self::new`] with an image of `mib` MiB (sparse).
    pub fn sized(tag: &str, mib: u64) -> Option<LoopFs> {
        let dir = std::env::temp_dir().join(format!("wol-loop-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mnt = dir.join("mnt");
        std::fs::create_dir_all(&mnt).ok()?;
        let img = dir.join("fs.img");
        let me = LoopFs { dir, mnt };
        let ok = std::fs::File::create(&img)
            .and_then(|f| f.set_len(mib << 20))
            .is_ok()
            && sudo(&["mkfs.btrfs", "-q", "-f", &img.to_string_lossy()])
            && sudo(&[
                "mount",
                "-o",
                "loop",
                &img.to_string_lossy(),
                &me.mnt.to_string_lossy(),
            ])
            && sudo(&[
                "chown",
                &format!("{}:{}", caller_uid(), caller_gid()),
                &me.mnt.to_string_lossy(),
            ]);
        if ok {
            return Some(me);
        }
        drop(me);
        assert!(
            std::env::var_os("WOL_REQUIRE_LOOP").is_none(),
            "WOL_REQUIRE_LOOP is set and a loop-mounted btrfs could not be made (sudo -n, mkfs.btrfs)"
        );
        eprintln!("SKIPPED: no loop-mounted btrfs here (sudo -n, mkfs.btrfs)");
        None
    }
    pub fn path(&self) -> &Path {
        &self.mnt
    }
}

/// The caller's ids from /proc (no libc call in a test support file).
fn caller_uid() -> String {
    status_id("Uid:")
}
fn caller_gid() -> String {
    status_id("Gid:")
}
fn status_id(key: &str) -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix(key)?
                    .split_whitespace()
                    .next()
                    .map(String::from)
            })
        })
        .unwrap_or_else(|| "0".into())
}

// Drop: unmounts the loop file system and deletes its image directory.
impl Drop for LoopFs {
    fn drop(&mut self) {
        let m = self.mnt.to_string_lossy().into_owned();
        // A file with fs-verity cannot be deleted by the owner on every kernel; the image goes
        // with its mount.
        sudo(&["umount", "-l", &m]);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
