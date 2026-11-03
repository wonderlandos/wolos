// SPDX-License-Identifier: GPL-2.0-or-later
//! The C boundary (ADR 0024 on ADR 0020's rules): opaque handle, UTF-8 with lengths, results as the
//! JSON of [`crate::Report`], panics stopped here, out-pointers written only on success.
use crate::{Engine, Limits, Report};
use std::ffi::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

pub const XGUARD_OK: i32 = 0;
pub const XGUARD_INVALID_ARGUMENT: i32 = 1;
pub const XGUARD_UNAVAILABLE: i32 = 3;
pub const XGUARD_INTERNAL: i32 = 4;

/// Run the signature rules too (spawns the worker; the plain library has only hashes and patterns).
pub const XGUARD_SCAN_DEEP: u32 = 1;

/// The worker the deep flag runs.
pub const WORKER: &str = "/usr/bin/wol-xguard";

/// Opaque. `magic` comes first (`repr(C)`) and is checked on every call before the engine is
/// used; drop overwrites it, so a released or foreign pointer is refused (audit B7).
#[repr(C)]
pub struct XguardEngine {
    magic: u64,
    engine: Engine,
}

const ENGINE_MAGIC: u64 = 0x5847_454e_4731_0001; // "XGENG1"
const DEAD: u64 = 0xdead_dead_dead_dead;

impl Drop for XguardEngine {
    fn drop(&mut self) {
        self.magic = DEAD;
    }
}

/// The engine behind a handle, or None for NULL, misaligned, released or foreign pointers.
///
/// # Safety
/// `e` is NULL or points at readable memory at least the size of a `u64`.
unsafe fn engine<'a>(e: *const XguardEngine) -> Option<&'a Engine> {
    if e.is_null() || !e.is_aligned() {
        return None;
    }
    // SAFETY: `magic` is the first field; it is read before the rest is trusted.
    if unsafe { std::ptr::read(e.cast::<u64>()) } != ENGINE_MAGIC {
        return None;
    }
    // SAFETY: the magic says this is a live engine.
    Some(unsafe { &(*e).engine })
}

/// The deep worker on the already opened `f` (audit S4-16): the guard's command (the file on
/// standard input, never a path the worker opens again or could read as a flag, no new
/// privileges, nobody when we are root) and its deadline; only that child is ever killed.
fn worker(
    exe: &Path,
    f: &std::fs::File,
    deadline: std::time::Duration,
) -> crate::guard::DeepOutcome {
    let streamed = f.metadata().is_ok_and(|m| m.len() > crate::guard::WHOLE);
    let root = shared::sys::euid() == 0;
    let c = crate::guard::deep_command_as(exe, &crate::rules_dir(), streamed, root);
    crate::guard::run_deep(c, f, deadline)
}

/// One file, with or without the signature rules. The Rust API of what the C calls do.
pub fn scan_path(e: &Engine, path: &str, deep: bool) -> Result<Report, String> {
    scan_path_with(
        e,
        path,
        deep,
        Path::new(WORKER),
        crate::guard::DEEP_DEADLINE,
    )
}

/// [`scan_path`] with the worker and its deadline named. A deep check that ran past the deadline
/// is Err (`XGUARD_UNAVAILABLE`): the caller must not take a hashes-only result for a deep one.
/// A worker that is absent or failed falls back to hashes and patterns, as before.
fn scan_path_with(
    e: &Engine,
    path: &str,
    deep: bool,
    exe: &Path,
    deadline: std::time::Duration,
) -> Result<Report, String> {
    use std::io::{Seek, SeekFrom};
    use std::os::unix::fs::OpenOptionsExt;
    // A FIFO does not block the open and a terminal is never taken as ours (S4-2).
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(shared::sys::OPEN_QUIET)
        .open(path)
        .map_err(|m| m.to_string())?;
    if deep {
        match worker(exe, &f, deadline) {
            crate::guard::DeepOutcome::Report(r) => return Ok(r),
            crate::guard::DeepOutcome::TimedOut => {
                return Err(format!(
                    "the deep check took longer than {} s",
                    deadline.as_secs()
                ));
            }
            crate::guard::DeepOutcome::Failed => {
                f.seek(SeekFrom::Start(0)).map_err(|m| m.to_string())?;
            }
        }
    }
    crate::scan_file(e, &mut f, path, None, Limits::default()).map_err(|m| m.to_string())
}

/// Longest path or name accepted across the boundary (PATH_MAX is 4096; names are shorter).
const MAX_TEXT: usize = 64 * 1024;

#[allow(clippy::disallowed_methods)] // the C boundary wrapper (clippy.toml)
unsafe fn text<'a>(p: *const c_char, len: usize) -> Option<&'a str> {
    // from_raw_parts is undefined above isize::MAX bytes; a path or name is never near that.
    if p.is_null() || len > MAX_TEXT {
        return None;
    }
    // SAFETY: the caller promises `len` readable bytes.
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(p.cast(), len) }).ok()
}

fn guard(f: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(XGUARD_INTERNAL)
}

unsafe fn give(s: String, out: *mut *mut c_char, out_len: *mut usize) -> i32 {
    let b = s.into_bytes().into_boxed_slice();
    let n = b.len();
    // SAFETY: both out-pointers were checked by the caller of this helper.
    unsafe {
        *out = Box::into_raw(b).cast();
        *out_len = n;
    }
    XGUARD_OK
}

/// ABI major; `libxguard.so.1`.
#[unsafe(no_mangle)]
pub extern "C" fn xguard_abi_version() -> u32 {
    1
}

/// Open the signatures in `dir` (NULL: the machine's newest).
///
/// # Safety
/// `dir` is NULL or `dir_len` readable bytes; `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xguard_engine_create(
    dir: *const c_char,
    dir_len: usize,
    out: *mut *mut XguardEngine,
) -> i32 {
    guard(|| {
        if out.is_null() {
            return XGUARD_INVALID_ARGUMENT;
        }
        let path = if dir.is_null() {
            crate::rules_dir()
        } else {
            // SAFETY: per this function's contract.
            match unsafe { text(dir, dir_len) } {
                Some(d) => d.into(),
                None => return XGUARD_INVALID_ARGUMENT,
            }
        };
        match Engine::open(Path::new(&path)) {
            Ok(e) => {
                // SAFETY: checked non-null above.
                unsafe {
                    *out = Box::into_raw(Box::new(XguardEngine {
                        magic: ENGINE_MAGIC,
                        engine: e,
                    }))
                };
                XGUARD_OK
            }
            Err(_) => XGUARD_UNAVAILABLE,
        }
    })
}

/// # Safety
/// `e` is NULL or came from [`xguard_engine_create`], and is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xguard_engine_release(e: *mut XguardEngine) {
    // SAFETY: per this function's contract; the magic is checked first.
    if unsafe { engine(e) }.is_some() {
        // SAFETY: per this function's contract.
        drop(unsafe { Box::from_raw(e) });
    }
}

/// Scan a file and what is packed inside it, in memory. `*json` is a [`Report`] document for
/// [`xguard_string_release`].
///
/// # Safety
/// `e` is a live engine, `path` has `path_len` readable bytes, the out-pointers are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xguard_scan_path(
    e: *const XguardEngine,
    path: *const c_char,
    path_len: usize,
    flags: u32,
    json: *mut *mut c_char,
    json_len: *mut usize,
) -> i32 {
    guard(|| {
        // SAFETY: per this function's contract.
        let Some(p) = (unsafe { text(path, path_len) }) else {
            return XGUARD_INVALID_ARGUMENT;
        };
        if e.is_null() || json.is_null() || json_len.is_null() {
            return XGUARD_INVALID_ARGUMENT;
        }
        // SAFETY: per the contract; the magic is checked before use.
        let Some(e) = (unsafe { engine(e) }) else {
            return XGUARD_INVALID_ARGUMENT;
        };
        match scan_path(e, p, flags & XGUARD_SCAN_DEEP != 0).map(|r| serde_json::to_string(&r)) {
            // SAFETY: checked non-null above.
            Ok(Ok(s)) => unsafe { give(s, json, json_len) },
            Ok(Err(_)) => XGUARD_INTERNAL,
            Err(_) => XGUARD_UNAVAILABLE,
        }
    })
}

#[allow(clippy::disallowed_methods)] // the C boundary wrapper (clippy.toml)
/// Scan bytes already in memory, and what is packed inside them.
///
/// # Safety
/// As [`xguard_scan_path`]; `data` has `len` readable bytes, `name` has `name_len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xguard_scan_memory(
    e: *const XguardEngine,
    data: *const u8,
    len: usize,
    name: *const c_char,
    name_len: usize,
    json: *mut *mut c_char,
    json_len: *mut usize,
) -> i32 {
    guard(|| {
        // SAFETY: per this function's contract.
        let Some(n) = (unsafe { text(name, name_len) }) else {
            return XGUARD_INVALID_ARGUMENT;
        };
        if e.is_null()
            || data.is_null()
            || json.is_null()
            || json_len.is_null()
            || len > isize::MAX.unsigned_abs()
        {
            return XGUARD_INVALID_ARGUMENT;
        }
        // SAFETY: non-null and sized per the contract.
        let Some(e) = (unsafe { engine(e) }) else {
            return XGUARD_INVALID_ARGUMENT;
        };
        // SAFETY: non-null and sized per the contract.
        let d = unsafe { std::slice::from_raw_parts(data, len) };
        let r = crate::scan_bytes(e, d, n, None, Limits::default());
        match serde_json::to_string(&r) {
            // SAFETY: checked non-null above.
            Ok(s) => unsafe { give(s, json, json_len) },
            Err(_) => XGUARD_INTERNAL,
        }
    })
}

/// # Safety
/// `s` and `len` came from one scan call, once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xguard_string_release(s: *mut c_char, len: usize) {
    if !s.is_null() {
        // SAFETY: built from a boxed slice of this length in `give`.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(s.cast::<u8>(), len)) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Builder, Table};
    use crate::scan::tests::EICAR;
    use sha2::{Digest, Sha256};

    fn rules() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("xguard-ffi-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut b = Builder::default();
        b.hash(Table::Sha256, 68, &Sha256::digest(EICAR), "Test.File");
        std::fs::write(d.join("a.xgdb"), b.build(3, 1)).unwrap();
        d
    }

    #[test]
    fn a_foreign_pointer_is_not_an_engine() {
        let fake = [0u64; 8];
        let (mut json, mut n) = (std::ptr::null_mut(), 0);
        // SAFETY: the pointer is readable; the call must refuse it by its magic.
        let rc = unsafe {
            xguard_scan_memory(
                fake.as_ptr().cast(),
                b"ok".as_ptr(),
                2,
                c"m".as_ptr(),
                1,
                &mut json,
                &mut n,
            )
        };
        assert_eq!(rc, XGUARD_INVALID_ARGUMENT);
    }

    #[allow(clippy::disallowed_methods)] // the C boundary wrapper (clippy.toml)
    #[test]
    fn create_scan_release_through_the_c_calls() {
        let d = rules();
        let dir = d.to_str().unwrap();
        let mut e = std::ptr::null_mut();
        let p = d.join("sample");
        std::fs::write(&p, EICAR).unwrap();
        let path = p.to_str().unwrap();
        let (mut json, mut n) = (std::ptr::null_mut(), 0);
        // SAFETY: valid pointers throughout; this is what a C caller does.
        unsafe {
            assert_eq!(
                xguard_engine_create(dir.as_ptr().cast(), dir.len(), &mut e),
                XGUARD_OK
            );
            assert_eq!(
                xguard_scan_path(e, path.as_ptr().cast(), path.len(), 0, &mut json, &mut n),
                XGUARD_OK
            );
            let doc = std::slice::from_raw_parts(json.cast::<u8>(), n);
            let v: serde_json::Value = serde_json::from_slice(doc).unwrap();
            assert_eq!(v["verdict"], "found");
            assert_eq!(v["signatures_versions"][0], 3);
            xguard_string_release(json, n);
            assert_eq!(
                xguard_scan_memory(e, b"ok".as_ptr(), 2, c"m".as_ptr(), 1, &mut json, &mut n),
                XGUARD_OK
            );
            let doc = std::slice::from_raw_parts(json.cast::<u8>(), n);
            let v: serde_json::Value = serde_json::from_slice(doc).unwrap();
            assert_eq!(v["verdict"], "clean");
            xguard_string_release(json, n);
            let missing = b"/nonexistent/x";
            assert_eq!(
                xguard_scan_path(e, missing.as_ptr().cast(), 14, 0, &mut json, &mut n),
                XGUARD_UNAVAILABLE
            );
            xguard_engine_release(e);
            xguard_engine_release(std::ptr::null_mut());
        }
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn bad_arguments_are_refused() {
        let mut e = std::ptr::null_mut();
        let (mut json, mut n) = (std::ptr::null_mut(), 0);
        // SAFETY: only null or invalid-text arguments, which are checked before use.
        unsafe {
            assert_eq!(
                xguard_engine_create(c"/nonexistent".as_ptr(), 12, &mut e),
                XGUARD_UNAVAILABLE
            );
            assert_eq!(
                xguard_engine_create(c"x".as_ptr(), 1, std::ptr::null_mut()),
                XGUARD_INVALID_ARGUMENT
            );
            assert_eq!(
                xguard_scan_path(std::ptr::null(), c"x".as_ptr(), 1, 0, &mut json, &mut n),
                XGUARD_INVALID_ARGUMENT
            );
            assert_eq!(
                xguard_engine_create(b"\xff".as_ptr().cast(), 1, &mut e),
                XGUARD_INVALID_ARGUMENT
            );
        }
    }

    #[test]
    fn a_missing_worker_is_no_verdict() {
        let f = std::fs::File::open("/etc/passwd").unwrap();
        let o = worker(
            Path::new("/nonexistent/wol-xguard"),
            &f,
            std::time::Duration::from_secs(5),
        );
        assert!(matches!(o, crate::guard::DeepOutcome::Failed), "{o:?}");
    }

    /// Exports, header and linker map list the same calls.
    #[test]
    fn abi_files_agree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let names = |t: String| -> std::collections::BTreeSet<String> {
            t.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .filter(|w| {
                    w.starts_with("xguard_") && !["xguard_engine", "xguard_status"].contains(w)
                })
                .map(str::to_owned)
                .collect()
        };
        let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap();
        let src: String = read("src/ffi.rs")
            .lines()
            .filter(|l| l.contains("extern \"C\" fn"))
            .collect::<Vec<_>>()
            .join("\n");
        let exports = names(src);
        assert_eq!(exports.len(), 6);
        assert_eq!(exports, names(read("abi/xguard.map")));
        assert_eq!(exports, names(read("include/xguard.h")));
    }

    /// Audit S4-16: the deep worker of the library has the guard's deadline. One that hangs is
    /// stopped and the call fails (XGUARD_UNAVAILABLE), never a hashes-only "clean"; one that
    /// cannot start falls back to hashes and patterns.
    #[test]
    fn a_hung_deep_worker_is_stopped_and_the_call_fails() {
        // Its own directory: rules() is shared with tests that rewrite and remove it.
        let d = std::env::temp_dir().join(format!("xguard-ffi-deep-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut b = Builder::default();
        b.hash(Table::Sha256, 68, &Sha256::digest(EICAR), "Test.File");
        std::fs::write(d.join("a.xgdb"), b.build(3, 1)).unwrap();
        let e = Engine::open(&d).unwrap();
        let sample = d.join("deep-sample");
        std::fs::write(&sample, b"plain text").unwrap();
        // A worker that never finishes when asked to scan.
        use std::os::unix::fs::PermissionsExt;
        let hang = d.join("hang-worker");
        std::fs::write(
            &hang,
            "#!/bin/sh\n[ \"$1\" = scan ] || exit 0\nexec sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&hang, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A file just written can refuse to start (ETXTBSY) while another test thread forks with
        // its descriptor; that ends when the fork execs.
        let runs = (0..200).any(|_| std::process::Command::new(&hang).status().is_ok());
        assert!(runs, "the fake worker never started");
        let p = sample.to_str().unwrap();
        let t = std::time::Instant::now();
        let r = scan_path_with(&e, p, true, &hang, std::time::Duration::from_millis(300));
        assert!(r.is_err(), "{r:?}");
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            t.elapsed()
        );
        let gone = d.join("no-such-worker");
        let r = scan_path_with(&e, p, true, &gone, std::time::Duration::from_secs(5)).unwrap();
        assert!(r.findings.is_empty());
        // A path that looks like a flag is a file name to us; the worker never sees it.
        let flag = d.join("--rules");
        std::fs::write(&flag, EICAR).unwrap();
        let r = scan_path_with(
            &e,
            flag.to_str().unwrap(),
            false,
            &gone,
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(r.findings[0].name, "Test.File");
        let _ = std::fs::remove_dir_all(&d);
    }
}
