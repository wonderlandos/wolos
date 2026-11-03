// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_engine_*`: the engine handle, dispatch and the event stream.

use super::{
    borrow, bytes_out, give, guard, guard_status, handle, into_raw, need, put, quiet, release,
    retain, text_in, wolkit_error, wolkit_status,
};
use crate::engine::Engine;
use crate::error::{Error, Status};
use std::ffi::{c_char, c_int};
use std::sync::Arc;

#[repr(C)]
#[allow(non_camel_case_types)] // the C name
pub struct wolkit_engine {
    tag: handle::Tag<wolkit_engine>,
    pub(crate) engine: Arc<Engine>,
}

handle::tagged!(wolkit_engine, 0x57_4f_4c_4b_49_54_45_01);

/// From the environment: whether `WOLKIT_BACKEND=native` opts in to the native backend, and the
/// session it runs in (`LC_ALL`, `LC_CTYPE` or `LANG`, the first set; `XDG_ACTIVATION_TOKEN`).
fn native_settings(
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> (bool, crate::window::native::Session) {
    let text = |k: &str| {
        var(k)
            .and_then(|v| v.into_string().ok())
            .filter(|v| !v.is_empty())
    };
    let native = text("WOLKIT_BACKEND").is_some_and(|v| v == "native");
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|k| text(k))
        .unwrap_or_default();
    let session = crate::window::native::Session {
        locale,
        activation_token: text("XDG_ACTIVATION_TOKEN"),
    };
    (native, session)
}

/// The font roots and index cache an engine made with `flags` uses (the environment as a lookup):
/// a headless engine reads only the pinned folder (`WOLKIT_FONT_DIR` overrides it), any other the
/// person's font folders.
fn font_set(
    flags: u32,
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> (Vec<std::path::PathBuf>, std::path::PathBuf) {
    use crate::text::measure::{pinned_font_roots, pinned_index_path};
    use crate::text::system::{default_font_roots, default_index_path};
    let index = default_index_path(&var);
    if flags & crate::abi::ENGINE_HEADLESS != 0 {
        (pinned_font_roots(&var), pinned_index_path(&index))
    } else {
        (default_font_roots(&var), index)
    }
}

/// `WOLKIT_VERSION_MAJOR << 16 | MINOR << 8 | PATCH`.
#[unsafe(no_mangle)]
pub extern "C" fn wolkit_version() -> u32 {
    (crate::VERSION.0 << 16) | (crate::VERSION.1 << 8) | crate::VERSION.2
}

#[unsafe(no_mangle)]
pub extern "C" fn wolkit_version_string() -> *const c_char {
    c"1.0.0".as_ptr()
}

/// # Safety
/// `app_id` points at `len` readable bytes; `out` is writable; `error` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_create(
    app_id: *const c_char,
    len: usize,
    flags: u32,
    out: *mut *mut wolkit_engine,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        need(out, "out")?;
        // SAFETY: the pointer and length are the caller's (contract).
        let id = unsafe { text_in(app_id, len, 128, "The app id") }?;
        let engine = Engine::new(id, flags)?;
        // The one place the library reads the process environment (RR-027 baseline row).
        let var = |k: &str| std::env::var_os(k);
        if let Some(path) = crate::window::kit::socket_path(var) {
            engine.set_kit_socket(&path);
        }
        // The native backend, only when asked for (it is not the default in this release), and
        // what it needs from the session: the locale for compose and the launcher's token.
        let (native, session) = native_settings(var);
        crate::window::native::set_opt_in(native);
        crate::window::native::set_session(session);
        // The fonts, indexed once per process (the first engine wins): the person's folders, or
        // for a headless engine only the pinned folder, so its screenshots match on every machine.
        let (roots, index) = font_set(flags, var);
        crate::text::measure::install(&roots, &index);
        let h = Arc::new(wolkit_engine {
            tag: handle::Tag::new(),
            engine,
        });
        // SAFETY: checked non-NULL above.
        unsafe { *out = into_raw(h) };
        Ok(())
    })
}

/// # Safety
/// `engine` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_retain(engine: *mut wolkit_engine) -> *mut wolkit_engine {
    // SAFETY: NULL or a live handle (contract).
    unsafe { retain(engine) }
}

/// # Safety
/// `engine` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_release(engine: *mut wolkit_engine) {
    // SAFETY: NULL or a reference the caller owns (contract).
    unsafe { release(engine) }
}

/// # Safety
/// `engine` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_get_fd(engine: *const wolkit_engine) -> c_int {
    // SAFETY: NULL or live (contract).
    quiet(-1, || {
        unsafe { borrow(engine) }.map_or(-1, |e| e.engine.fd())
    })
}

/// # Safety
/// `engine` is live; `flags` is writable or NULL; `error` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_dispatch(
    engine: *mut wolkit_engine,
    timeout_ms: i32,
    flags: *mut u32,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        // SAFETY: live (contract).
        let e = unsafe { borrow(engine) }?;
        let f = e.engine.dispatch(timeout_ms)?;
        put(flags, f);
        Ok(())
    })
}

/// # Safety
/// `engine` is live; `buf` has `cap` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_read_events(
    engine: *mut wolkit_engine,
    buf: *mut u8,
    cap: usize,
) -> usize {
    quiet(0, || {
        // SAFETY: live (contract).
        let Ok(e) = (unsafe { borrow(engine) }) else {
            return 0;
        };
        // SAFETY: `cap` writable bytes (contract).
        let Ok(out) = (unsafe { bytes_out(buf, cap, "The event buffer") }) else {
            return 0;
        };
        e.engine.read_events(out)
    })
}

/// # Safety
/// `engine` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_get_info(engine: *const wolkit_engine, key: u32) -> u64 {
    // SAFETY: NULL or live (contract).
    quiet(0, || {
        unsafe { borrow(engine) }.map_or(0, |e| e.engine.info(key))
    })
}

/// # Safety
/// `engine` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_trim(engine: *mut wolkit_engine, level: u32) {
    // SAFETY: NULL or live (contract).
    quiet((), || {
        if let Ok(e) = unsafe { borrow(engine) } {
            e.engine.trim(level);
        }
    })
}

/// # Safety
/// `engine` is live; `buf` has `cap` writable bytes; `needed` and `error` are writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_stats_json(
    engine: *mut wolkit_engine,
    buf: *mut u8,
    cap: usize,
    needed: *mut usize,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        // SAFETY: live (contract).
        let e = unsafe { borrow(engine) }?;
        let json = e.engine.stats_json();
        // SAFETY: the buffer is the caller's (contract).
        unsafe { give(buf, cap, needed, json.as_bytes(), "The statistics") }
    })
}

/// # Safety
/// `engine` is live; `out16` has 16 writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_get_gpu_uuid(
    engine: *const wolkit_engine,
    out16: *mut u8,
) -> wolkit_status {
    guard_status(|| {
        // SAFETY: live (contract).
        let _ = unsafe { borrow(engine) }?;
        let _ = out16;
        Err(Error::new(
            Status::Unavailable,
            "This engine has no GPU; it draws on the CPU.",
        ))
    })
}

/// # Safety
/// `engine` is live; `error` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_set_environment(
    engine: *mut wolkit_engine,
    dark: u32,
    accent_rgb: u32,
    flags: u32,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        // SAFETY: live (contract).
        let e = unsafe { borrow(engine) }?;
        e.engine.set_environment(dark != 0, accent_rgb, flags)
    })
}

#[cfg(test)]
mod tests {
    use super::{font_set, native_settings};
    use std::ffi::OsString;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn native_is_off_unless_the_environment_asks_for_it() {
        assert!(!native_settings(env(&[])).0);
        assert!(!native_settings(env(&[("WOLKIT_BACKEND", "kit")])).0);
        assert!(native_settings(env(&[("WOLKIT_BACKEND", "native")])).0);
    }

    #[test]
    fn the_session_takes_the_first_locale_set_and_the_launch_token() {
        let (_, s) = native_settings(env(&[
            ("LANG", "de_DE.UTF-8"),
            ("LC_CTYPE", "fr_FR.UTF-8"),
            ("LC_ALL", ""),
            ("XDG_ACTIVATION_TOKEN", "tok"),
        ]));
        assert_eq!(s.locale, "fr_FR.UTF-8");
        assert_eq!(s.activation_token.as_deref(), Some("tok"));
        let (_, none) = native_settings(env(&[]));
        assert!(none.locale.is_empty() && none.activation_token.is_none());
    }

    #[test]
    fn a_headless_engine_reads_only_the_pinned_font_folder() {
        use crate::text::measure::PINNED_FONT_DIR;
        use std::path::PathBuf;
        let home = [("HOME", "/home/p")];
        let (roots, index) = font_set(crate::abi::ENGINE_HEADLESS, env(&home));
        assert_eq!(roots, vec![PathBuf::from(PINNED_FONT_DIR)]);
        assert_eq!(
            index,
            PathBuf::from("/home/p/.cache/wolkit/fonts-pinned.idx")
        );
        let over = [("HOME", "/home/p"), ("WOLKIT_FONT_DIR", "/ci/inter")];
        let (roots, _) = font_set(crate::abi::ENGINE_HEADLESS, env(&over));
        assert_eq!(roots, vec![PathBuf::from("/ci/inter")]);
        // Any other engine keeps the person's folders and ignores the override.
        let (roots, index) = font_set(0, env(&over));
        assert!(
            roots.contains(&PathBuf::from("/home/p/.local/share/fonts")),
            "{roots:?}"
        );
        assert!(!roots.contains(&PathBuf::from("/ci/inter")));
        assert_eq!(index, PathBuf::from("/home/p/.cache/wolkit/fonts.idx"));
    }
}
