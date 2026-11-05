// SPDX-License-Identifier: BSD-2-Clause
//! fs-verity for the seal (`wol_core::seal::verity`), in this process: `FS_IOC_ENABLE_VERITY` and `FS_IOC_MEASURE_VERITY`
//! on a descriptor, with what `fsverity enable` uses by default (SHA-256, 4096-byte blocks, no
//! salt, no signature). `wol seal apply` used to start `fsverity` three times per file (a
//! measure, an enable, a measure) on one thread: 71,385 files took 14 minutes of process
//! creation (IMG013-3-SEAL-SLOW), and a kill of one `fsverity measure` after a clock jump was
//! reported as "no fs-verity digest" (SEAL-FLAKE). The two ioctls on four threads do 39,000 files
//! a second on the same disk (docs/modules/seal.md has the table).
//!
//! [`seal_all`] answers in the order of its input, so the caller's manifest does not depend on
//! which thread was first. A failure says which step failed and the operating system's own
//! reason; nothing here waits on a child, so there is no time-out to mistake for an answer.
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Worker threads. Measured (IMG013-3): 4 threads 39,000 files/s, 8 threads 37,000, 16 threads
/// 6,300: past about 8 the file system's own locks make it slower.
pub const THREADS: usize = 4;

/// `FS_VERITY_HASH_ALG_SHA256` in `<linux/fsverity.h>`.
const HASH_SHA256: u32 = 1;
/// The block size `fsverity enable` uses unless told otherwise.
const BLOCK_SIZE: u32 = 4096;
/// `_IOW('f', 133, struct fsverity_enable_arg)`: the struct is 128 bytes.
const FS_IOC_ENABLE_VERITY: u64 = 0x4080_6685;
/// `_IOWR('f', 134, struct fsverity_digest)`: the header is 4 bytes.
const FS_IOC_MEASURE_VERITY: u64 = 0xC004_6686;

/// `struct fsverity_enable_arg` (version 1), every field the kernel reads.
#[repr(C)]
struct EnableArg {
    version: u32,
    hash_algorithm: u32,
    block_size: u32,
    salt_size: u32,
    salt_ptr: u64,
    sig_size: u32,
    reserved1: u32,
    sig_ptr: u64,
    reserved2: [u64; 11],
}

/// `struct fsverity_digest` with room for a SHA-512 digest (the kernel's largest).
#[repr(C)]
struct Digest {
    algorithm: u16,
    size: u16,
    bytes: [u8; 64],
}

/// The step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The file could not be opened (a link, a permission, gone).
    Open,
    /// The kernel refused to turn fs-verity on (the file system has none, the file is open for
    /// writing, the file is too large for the tree).
    Enable,
    /// The kernel would not tell the digest, or told one that is not SHA-256.
    Measure,
}

/// Why a file has no digest: the step and the system's reason.
#[derive(Debug)]
pub struct Fail {
    pub stage: Stage,
    pub err: io::Error,
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.stage {
            Stage::Open => "could not be opened",
            Stage::Enable => "fs-verity could not be enabled",
            Stage::Measure => "fs-verity digest could not be read",
        };
        write!(f, "{what}: {}", self.err)
    }
}

fn retry<T>(mut f: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    loop {
        match f() {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            r => return r,
        }
    }
}

fn enable_fd(fd: i32) -> io::Result<()> {
    let arg = EnableArg {
        version: 1,
        hash_algorithm: HASH_SHA256,
        block_size: BLOCK_SIZE,
        salt_size: 0,
        salt_ptr: 0,
        sig_size: 0,
        reserved1: 0,
        sig_ptr: 0,
        reserved2: [0; 11],
    };
    // SAFETY: `fd` is an open descriptor of the caller and `arg` is a live `EnableArg` the
    // kernel only reads (`_IOW`), with the layout of `struct fsverity_enable_arg`.
    let r = unsafe { libc::ioctl(fd, FS_IOC_ENABLE_VERITY as _, &arg) };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn measure_fd(fd: i32) -> io::Result<String> {
    let mut d = Digest {
        algorithm: 0,
        size: 64,
        bytes: [0; 64],
    };
    // SAFETY: `fd` is an open descriptor of the caller and `d` is a live `Digest` with room for
    // `size` bytes after its 4-byte header, which is all the kernel writes (`_IOWR`).
    let r = unsafe { libc::ioctl(fd, FS_IOC_MEASURE_VERITY as _, &mut d) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    if u32::from(d.algorithm) != HASH_SHA256 || d.size != 32 {
        return Err(io::Error::other(format!(
            "the digest is not SHA-256 (algorithm {}, {} bytes)",
            d.algorithm, d.size
        )));
    }
    Ok(d.bytes[..32].iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether a measure answered "no fs-verity on this file" (`ENODATA`), as opposed to failing.
fn is_not_enabled(e: &io::Error) -> bool {
    e.raw_os_error() == Some(libc::ENODATA)
}

/// The fs-verity digest of `path` (hex SHA-256), turning fs-verity on first when the file has
/// none. A file that has it already (sealed before, or by a link of the same inode: `EEXIST` from
/// an enable that lost a race) answers with its digest. Never follows a final symlink.
pub fn seal_file(path: &Path) -> Result<String, Fail> {
    let fail = |stage, err| Fail { stage, err };
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| fail(Stage::Open, e))?;
    let fd = file.as_raw_fd();
    // Sealed already (an earlier run, another link of the inode): its digest stands. Any other
    // answer (not enabled yet, or a file system that answers nothing) is for the enable to name.
    if let Ok(d) = retry(|| measure_fd(fd)) {
        return Ok(d);
    }
    match retry(|| enable_fd(fd)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(fail(Stage::Enable, e)),
    }
    retry(|| measure_fd(fd)).map_err(|e| {
        let e = if is_not_enabled(&e) {
            io::Error::other("the file has no fs-verity after it was enabled")
        } else {
            e
        };
        fail(Stage::Measure, e)
    })
}

/// [`seal_file`] of every path on `threads` workers. The answer has one entry per input path, in
/// the input's order; after the first failure the workers stop taking new paths, so an entry is
/// `None` only after a failed one (the first `None`-or-`Err` in order is always an `Err`).
pub fn seal_all(paths: &[PathBuf], threads: usize) -> Vec<Option<Result<String, Fail>>> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let out: Mutex<Vec<Option<Result<String, Fail>>>> =
        Mutex::new((0..paths.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..threads.clamp(1, paths.len().max(1)) {
            s.spawn(|| {
                let mut mine = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(p) = paths.get(i) else { break };
                    let r = seal_file(p);
                    if r.is_err() {
                        stop.store(true, Ordering::Relaxed);
                    }
                    mine.push((i, r));
                }
                let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
                for (i, r) in mine {
                    o[i] = Some(r);
                }
            });
        }
    });
    out.into_inner().unwrap_or_else(|e| e.into_inner())
}
