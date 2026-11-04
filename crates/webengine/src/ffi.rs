// SPDX-License-Identifier: BSD-2-Clause
//! The boundary to `libwolwebengine.so.1` (`engine/include/wolwebengine.h`, ABI 1): the C types,
//! mirrored, and the library loaded once per process with `libloading` (the soname, or the path
//! the first [`load`] names). Nothing links the engine: a machine without the package still builds and
//! runs every other part of WolOS, and a web app reports the missing library as an error.
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::sync::OnceLock;

pub const ABI: u32 = 1;
/// What is loaded unless [`load`] named a path first.
pub const SONAME: &str = "libwolwebengine.so.1";
/// The variable the tests and the package's check() name a build tree's library with (the
/// binary reads it and calls [`load`]; the library never reads the environment).
pub const LIB_ENV: &str = "WOL_WEBENGINE_LIB";

pub const MULTIPROCESS: u32 = 1;
/// The content process confines itself (the library refuses [`MULTIPROCESS`] without it).
pub const SANDBOX: u32 = 2;
pub const FEATURE_WEBGL2: u32 = 1;
pub const FEATURE_WEBGPU: u32 = 2;
pub const FEATURE_SERVICE_WORKER: u32 = 4;
pub const FEATURE_INDEXEDDB: u32 = 8;
pub const FEATURE_NOTIFICATIONS: u32 = 16;
pub const FEATURE_WEBRTC: u32 = 32;
pub const LOAD_NAVIGATION: u32 = 0;
pub const LOAD_RESOURCE: u32 = 1;
pub const TARGET_HEADLESS: u32 = 0;
pub const TARGET_WAYLAND: u32 = 1;
pub const INPUT_MOUSE_MOVE: u32 = 1;
pub const INPUT_MOUSE_BUTTON: u32 = 2;
pub const INPUT_WHEEL: u32 = 3;
pub const INPUT_KEY: u32 = 4;
pub const INPUT_IME_PREEDIT: u32 = 5;
pub const INPUT_IME_COMMIT: u32 = 6;
pub const INPUT_IME_DISMISS: u32 = 7;
pub const INPUT_MOUSE_LEAVE: u32 = 8;
pub const LOAD_COMPLETE: u32 = 3;

// SAFETY: the callbacks the engine calls with `Config::ctx` (header: `allow`, `wake`,
// `clip_get`, `clip_set`); `crate::view` implements them for its boxed context.
/// `wwe_allow_fn`: may the app load this URL (kind [`LOAD_NAVIGATION`] or [`LOAD_RESOURCE`]).
pub type AllowFn = unsafe extern "C" fn(*mut c_void, *const u8, usize, u32) -> bool;
/// `wwe_wake_fn`: the engine has work (any thread).
pub type WakeFn = unsafe extern "C" fn(*mut c_void);
// SAFETY: as above (the clipboard pair).
/// `wwe_clip_get_fn`: copy the clipboard text, return its full length or -1.
pub type ClipGetFn = unsafe extern "C" fn(*mut c_void, *mut u8, usize) -> isize;
/// `wwe_clip_set_fn`: the page copied this text.
pub type ClipSetFn = unsafe extern "C" fn(*mut c_void, *const u8, usize);

// SAFETY: `wwe_message_fn` (WWE_1.1): the host's callback for a page message, called on the engine
// thread with the `ctx` given to `wwe_engine_set_host_channel`.
pub type MessageFn = unsafe extern "C" fn(*mut c_void, *const u8, usize);
// SAFETY: WWE_1.1 `wwe_engine_set_host_channel`: a live engine, valid strings, a callback and a context
// that live until the engine is freed, an error buffer.
pub type SetChannelFn = unsafe extern "C" fn(
    *mut EngineH,
    *const c_char,
    *const c_char,
    Option<MessageFn>,
    *mut c_void,
    *mut c_char,
    usize,
) -> i32;
// SAFETY: WWE_1.1 `wwe_view_post_message`: a live view and `len` readable bytes.
pub type PostMessageFn = unsafe extern "C" fn(*mut ViewH, *const u8, usize) -> i32;

#[repr(C)]
pub struct Config {
    pub abi: u32,
    pub flags: u32,
    pub features: u32,
    pub heap_mb: u32,
    pub profile_dir: *const c_char,
    pub user_agent: *const c_char,
    pub allow: Option<AllowFn>,
    pub wake: Option<WakeFn>,
    pub clip_get: Option<ClipGetFn>,
    pub clip_set: Option<ClipSetFn>,
    pub ctx: *mut c_void,
}

#[repr(C)]
pub struct TargetC {
    pub kind: u32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub wl_display: *mut c_void,
    pub wl_surface: *mut c_void,
}

#[repr(C)]
pub struct InputC {
    pub kind: u32,
    pub button: u32,
    pub pressed: u32,
    pub mods: u32,
    pub repeat: u32,
    pub x: f32,
    pub y: f32,
    pub dx: f32,
    pub dy: f32,
    pub key: *const c_char,
    pub code: *const c_char,
}

#[repr(C)]
#[derive(Default)]
pub struct State {
    pub load: u32,
    pub crashed: u32,
    pub frames: u64,
}

#[repr(C)]
pub struct ImageC {
    pub width: u32,
    pub height: u32,
    pub rgba: *mut u8,
}

pub enum EngineH {}
pub enum ViewH {}

/// The library's functions, resolved once. Each is called with the header's contract (ABI 1).
pub struct Lib {
    // SAFETY: version queries, no arguments.
    pub abi_version: unsafe extern "C" fn() -> u32,
    pub engine_version: unsafe extern "C" fn() -> *const c_char,
    // SAFETY: a valid config and error buffer; the engine freed once, on its own thread.
    pub engine_new: unsafe extern "C" fn(*const Config, *mut c_char, usize) -> *mut EngineH,
    pub engine_free: unsafe extern "C" fn(*mut EngineH),
    pub engine_spin: unsafe extern "C" fn(*mut EngineH),
    // SAFETY: a live engine, valid target and URL; the view freed once, before its engine.
    pub view_new: unsafe extern "C" fn(
        *mut EngineH,
        *const TargetC,
        *const c_char,
        *mut c_char,
        usize,
    ) -> *mut ViewH,
    // SAFETY: a live view, freed once; the state written into a valid out pointer.
    pub view_free: unsafe extern "C" fn(*mut ViewH),
    pub view_state: unsafe extern "C" fn(*const ViewH, *mut State),
    // SAFETY: a live view.
    pub view_resize: unsafe extern "C" fn(*mut ViewH, u32, u32, f32),
    pub view_input: unsafe extern "C" fn(*mut ViewH, *const InputC),
    pub view_focus: unsafe extern "C" fn(*mut ViewH, bool),
    pub view_visible: unsafe extern "C" fn(*mut ViewH, bool),
    // SAFETY: a live view; an image is freed once with image_free.
    pub view_theme: unsafe extern "C" fn(*mut ViewH, bool),
    pub view_screenshot:
        unsafe extern "C" fn(*mut ViewH, u32, *mut ImageC, *mut c_char, usize) -> i32,
    pub image_free: unsafe extern "C" fn(*mut ImageC),
    // SAFETY: a token from the engine's own command line.
    pub run_content_process: unsafe extern "C" fn(*const c_char),
    /// WWE_1.1 (ADR 0048), absent in a library of the 1.0 build: the page-to-host channel. Without
    /// them an app runs in development mode (the page has no `window.__wolosHost`).
    pub set_host_channel: Option<SetChannelFn>,
    pub post_message: Option<PostMessageFn>,
    /// The loaded library: the function pointers above live as long as it.
    _handle: libloading::Library,
}

/// `len` bytes at `p` as a slice: the one place this crate builds a slice from a C pointer (audit
/// B7, `clippy.toml`). An empty slice for a NULL `p` or a zero `len`.
///
/// # Safety
/// `p` is NULL or valid for reads of `len` bytes for `'a`, and not written during it.
pub unsafe fn bytes_in<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if p.is_null() || len == 0 {
        return &[];
    }
    // SAFETY: the caller's contract above; non-NULL and non-empty checked.
    #[allow(clippy::disallowed_methods)] // the boundary helper clippy.toml names
    unsafe {
        std::slice::from_raw_parts(p, len)
    }
}

fn open(path: &str) -> Result<Lib, String> {
    let flags = libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_LOCAL;
    // SAFETY: loading runs the library's initialisers; this is the pinned engine library (its
    // only constructors are the engine's own registrations). Kept for the life of the process.
    let handle: libloading::Library =
        unsafe { libloading::os::unix::Library::open(Some(path), flags) }
            .map_err(|e| format!("cannot load {path}: {e}"))?
            .into();
    macro_rules! f {
        ($name:literal) => {{
            // SAFETY: the symbol has the type the header (ABI 1) declares, mirrored in `Lib`.
            let sym = unsafe { handle.get($name) }.map_err(|e| format!("{path}: {e}"))?;
            *sym
        }};
    }
    // The 1.1 additions may be missing: that is a library of the earlier build, not an error.
    macro_rules! opt {
        ($name:literal) => {
            // SAFETY: the symbol, when present, has the type the header (ABI 1, WWE_1.1) declares.
            unsafe { handle.get($name) }.ok().map(|sym| *sym)
        };
    }
    let lib = Lib {
        abi_version: f!(b"wwe_abi_version"),
        engine_version: f!(b"wwe_engine_version"),
        engine_new: f!(b"wwe_engine_new"),
        engine_free: f!(b"wwe_engine_free"),
        engine_spin: f!(b"wwe_engine_spin"),
        view_new: f!(b"wwe_view_new"),
        view_free: f!(b"wwe_view_free"),
        view_state: f!(b"wwe_view_state"),
        view_resize: f!(b"wwe_view_resize"),
        view_input: f!(b"wwe_view_input"),
        view_focus: f!(b"wwe_view_focus"),
        view_visible: f!(b"wwe_view_visible"),
        view_theme: f!(b"wwe_view_theme"),
        view_screenshot: f!(b"wwe_view_screenshot"),
        image_free: f!(b"wwe_image_free"),
        run_content_process: f!(b"wwe_run_content_process"),
        set_host_channel: opt!(b"wwe_engine_set_host_channel"),
        post_message: opt!(b"wwe_view_post_message"),
        _handle: handle,
    };
    // SAFETY: resolved above with its declared type.
    let abi = unsafe { (lib.abi_version)() };
    if abi != ABI {
        return Err(format!("{path} has ABI {abi}, this build needs {ABI}"));
    }
    Ok(lib)
}

static LIB: OnceLock<Result<Lib, String>> = OnceLock::new();

/// Loads the library from `path` (a build tree), or the soname for `None`. The first call of
/// this or [`lib`] decides for the whole process; later calls return that result.
pub fn load(path: Option<&Path>) -> Result<&'static Lib, String> {
    let p = path.map_or_else(|| SONAME.to_owned(), |p| p.to_string_lossy().into_owned());
    LIB.get_or_init(|| open(&p)).as_ref().map_err(Clone::clone)
}

/// The library, loaded at the first call (the soname unless [`load`] came first).
pub fn lib() -> Result<&'static Lib, String> {
    load(None)
}

/// The pinned engine's version as the library reports it.
pub fn engine_version() -> Result<String, String> {
    let l = lib()?;
    // SAFETY: a static NUL-terminated string (header).
    Ok(unsafe { CStr::from_ptr((l.engine_version)()) }
        .to_string_lossy()
        .into_owned())
}

/// In the engine's content process (see [`crate::process::content_token`]): confines the process
/// (`src/engine/confine.rs`; it returns without running the page if it cannot), then runs the page until the engine ends it;
/// the host's `main` then returns. Call it first in `main`: the confinement needs the process to
/// have one thread.
pub fn run_content_process(token: &str) -> Result<(), String> {
    let l = lib()?;
    let t = CString::new(token).map_err(|_| "token has a NUL".to_string())?;
    // SAFETY: t is a valid C string for the call.
    unsafe { (l.run_content_process)(t.as_ptr()) };
    Ok(())
}
