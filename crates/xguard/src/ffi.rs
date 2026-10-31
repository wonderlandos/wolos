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

/// Opaque.
pub struct XguardEngine(Engine);

/// A report from the worker: its JSON line. None when the worker is absent or failed.
fn worker(exe: &str, path: &str, dir: Option<&str>) -> Option<Report> {
    let mut c = std::process::Command::new(exe);
    c.args(["scan", "--json"]);
    if let Some(d) = dir {
        c.args(["--rules", d]);
    }
    let out = c.arg(path).output().ok()?;
    serde_json::from_slice(out.stdout.split(|b| *b == b'\n').next()?).ok()
}

/// One file, with or without the signature rules. The Rust API of what the C calls do.
pub fn scan_path(e: &Engine, path: &str, deep: bool) -> Result<Report, String> {
    if deep && let Some(r) = worker(WORKER, path, None) {
        return Ok(r);
    }
    let mut f = std::fs::File::open(path).map_err(|m| m.to_string())?;
    crate::scan_file(e, &mut f, path, None, Limits::default()).map_err(|m| m.to_string())
}

unsafe fn text<'a>(p: *const c_char, len: usize) -> Option<&'a str> {
    if p.is_null() {
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
                unsafe { *out = Box::into_raw(Box::new(XguardEngine(e))) };
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
    if !e.is_null() {
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
        // SAFETY: non-null, live per the contract.
        let e = unsafe { &(*e).0 };
        match scan_path(e, p, flags & XGUARD_SCAN_DEEP != 0).map(|r| serde_json::to_string(&r)) {
            // SAFETY: checked non-null above.
            Ok(Ok(s)) => unsafe { give(s, json, json_len) },
            Ok(Err(_)) => XGUARD_INTERNAL,
            Err(_) => XGUARD_UNAVAILABLE,
        }
    })
}

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
        if e.is_null() || data.is_null() || json.is_null() || json_len.is_null() {
            return XGUARD_INVALID_ARGUMENT;
        }
        // SAFETY: non-null and sized per the contract.
        let (e, d) = unsafe { (&(*e).0, std::slice::from_raw_parts(data, len)) };
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
        assert!(worker("/nonexistent/wol-xguard", "/etc/hostname", None).is_none());
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
}
