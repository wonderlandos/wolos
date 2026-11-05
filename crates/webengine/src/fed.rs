// SPDX-License-Identifier: BSD-2-Clause
//! The host's `fed:` handler (WWE_1.2, ADR 0052): every web view this crate makes serves Federate
//! Network addresses natively, with no setting and no extension. The library calls [`load_cb`] on a
//! thread of its own for each `fed://` load and [`map_cb`] before each page load of an http(s) URL;
//! both answer from the `federate` crate, which verifies every signature and content address
//! before a byte is served and otherwise serves a page saying what failed. Which origins an app
//! may reach is still its policy's ([`crate::sandbox`]): `fed://name.tld` is an origin like
//! `https://host`.
use crate::ffi;
use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

/// The scheme, as the library's `wwe_engine_set_scheme` takes it.
pub const SCHEME: &std::ffi::CStr = c"fed";

static RESOLVER: OnceLock<federate::Resolver> = OnceLock::new();

/// The resolver `fed:` loads use in this process, set before the first view (tests serve a
/// network of their own with it); false when one is already in use. Without a call, the
/// `federate` crate's own: the pinned root key and the network's HTTPS door.
pub fn use_resolver(r: federate::Resolver) -> bool {
    RESOLVER.set(r).is_ok()
}

fn resolver() -> &'static federate::Resolver {
    RESOLVER.get().unwrap_or_else(federate::resolve::shared)
}

/// The reply for `url` loaded as `kind`; a panic in the resolver is a 502 page, never an unwind
/// into the library.
pub fn serve(url: &str, kind: federate::Load) -> federate::Reply {
    std::panic::catch_unwind(|| resolver().serve_load(url, kind)).unwrap_or_else(|_| {
        federate::Reply {
            status: 502,
            mime: "text/plain; charset=utf-8".into(),
            body: b"The Federate resolver failed.".to_vec(),
        }
    })
}

/// The `fed://` spelling of an `http(s)://name.tld` URL of the Federate Network, in this process's
/// resolver; `None` when `url` is not one (an ordinary web host, a TLD outside the network's).
pub fn map_http(url: &str) -> Option<String> {
    guarded(|| resolver().map_http(url))
}

/// `f`'s answer, or `None` when it panics: a panic must not unwind into the library (across
/// `extern "C"` that aborts the process).
fn guarded(f: impl FnOnce() -> Option<String>) -> Option<String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .ok()
        .flatten()
}

/// Copies `s` NUL-terminated into the `cap` bytes at `out`, truncated; the bytes copied.
///
/// # Safety
/// `out` is NULL or writable for `cap` bytes.
unsafe fn put_c(s: &str, out: *mut c_char, cap: usize) -> usize {
    if out.is_null() || cap == 0 {
        return 0;
    }
    let n = s.len().min(cap - 1);
    // SAFETY: contract above; n + 1 <= cap.
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), out.cast::<u8>(), n);
        out.add(n).write(0);
    }
    n
}

// SAFETY: `wwe_scheme_load_fn`: the library passes `len` readable bytes at `url`, `mimecap`
// writable bytes at `mime`, and a sink callable during this call; any thread.
pub(crate) unsafe extern "C" fn load_cb(
    _ctx: *mut c_void,
    url: *const u8,
    len: usize,
    kind: u32,
    mime: *mut c_char,
    mimecap: usize,
    body: ffi::BodyFn,
    sink: *mut c_void,
) -> u32 {
    // SAFETY: as above.
    let url = String::from_utf8_lossy(unsafe { ffi::bytes_in(url, len) }).into_owned();
    // WWE_LOAD_NAVIGATION is a document; anything else has the subresource cap.
    let kind = if kind == ffi::LOAD_NAVIGATION {
        federate::Load::Document
    } else {
        federate::Load::Subresource
    };
    let reply = serve(&url, kind);
    // SAFETY: as above: the buffer and the sink are the library's, for this call.
    unsafe {
        put_c(&reply.mime, mime, mimecap);
        body(sink, reply.body.as_ptr(), reply.body.len());
    }
    u32::from(reply.status)
}

// SAFETY: `wwe_scheme_map_fn`: `len` readable bytes at `url`, `cap` writable bytes at `out`.
pub(crate) unsafe extern "C" fn map_cb(
    _ctx: *mut c_void,
    url: *const u8,
    len: usize,
    out: *mut c_char,
    cap: usize,
) -> isize {
    // SAFETY: as above.
    let bytes = unsafe { ffi::bytes_in(url, len) };
    // A panic must not unwind into the library (it would abort the process): no mapping then.
    let Some(fed) = guarded(|| {
        std::str::from_utf8(bytes)
            .ok()
            .and_then(|u| resolver().map_http(u))
    }) else {
        return -1;
    };
    if fed.len() >= cap {
        return -1;
    }
    // SAFETY: as above; fed.len() < cap.
    let n = unsafe { put_c(&fed, out, cap) };
    isize::try_from(n).unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panicking_mapping_is_no_mapping() {
        assert_eq!(
            guarded(|| Some("fed://home.fed".into())).as_deref(),
            Some("fed://home.fed")
        );
        assert_eq!(guarded(|| panic!("resolver bug")), None);
    }
}
