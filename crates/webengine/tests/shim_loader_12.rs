// SPDX-License-Identifier: BSD-2-Clause
//! WWE_1.2 (ADR 0052) against the C boundary, with the stand-in library (`tests/mock_engine.c`
//! built with `-DSCHEME`; skipped where there is no C compiler): every view registers the host's
//! `fed:` handler with no setting, a `fed://` load through the library's callback is the verified
//! page, and `http://home.fed` is spelled `fed://home.fed` while other hosts stay on the web.
use federate::{Resolver, Source};
use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::process::Command;
use webengine::{App, Policy, Target, WebView, ffi};

const ROOT: &[u8] = include_bytes!("../../federate/tests/vectors/root-zone.json");
const MANIFEST: &[u8] = include_bytes!("../../federate/tests/vectors/manifest-home.fed.json");
const INDEX: &[u8] = include_bytes!("../../federate/tests/vectors/block-home.fed-index.html");

/// The network's data as captured (crates/federate/tests/chain.rs has the provenance).
struct Captured;
impl Source for Captured {
    fn get(&self, path: &str, _max: u64) -> Result<Vec<u8>, String> {
        match path {
            "/v1/root" => Ok(ROOT.to_vec()),
            "/v1/manifest/41220e29510791e1fbbc13dbf2ba610dc9598f28a0f102f488f20610780c4c6f" => {
                Ok(MANIFEST.to_vec())
            }
            "/v1/block/ddfe588ac4fe64292a561ae36aee15d32250b1bbc2b62fd8f84e09b96e254066" => {
                Ok(INDEX.to_vec())
            }
            _ => Err(format!("no {path}")),
        }
    }
}

fn mock_lib() -> Option<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_engine.c");
    let out = std::env::temp_dir().join(format!("wol-mock-engine-{}-{}", 12, std::process::id()));
    std::fs::create_dir_all(&out).ok()?;
    let so = out.join("libwolwebengine.so.1");
    let mut cc = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()));
    cc.args(["-shared", "-fPIC", "-DCHANNEL", "-DSCHEME", "-o"])
        .arg(&so)
        .arg(&src);
    cc.status().ok()?.success().then_some(so)
}

// SAFETY: the stand-in library's test hooks (mock_engine.c).
type LoadHook =
    unsafe extern "C" fn(*const c_char, u32, *mut c_char, usize, *mut u8, usize, *mut usize) -> u32;
type MapHook = unsafe extern "C" fn(*const c_char, *mut c_char, usize) -> isize;

struct Hooks {
    load: LoadHook,
    map: MapHook,
    _lib: libloading::Library,
}

impl Hooks {
    fn open(so: &std::path::Path) -> Hooks {
        // SAFETY: the stand-in library built above (no initialisers); the symbols have the types
        // mock_engine.c declares.
        unsafe {
            let lib = libloading::Library::new(so).unwrap();
            let load = *lib.get::<LoadHook>(b"mock_fed_load").unwrap();
            let map = *lib.get::<MapHook>(b"mock_fed_map").unwrap();
            Hooks {
                load,
                map,
                _lib: lib,
            }
        }
    }

    fn load(&self, url: &str) -> (u32, String, Vec<u8>) {
        self.load_as(url, ffi::LOAD_NAVIGATION)
    }

    fn load_as(&self, url: &str, kind: u32) -> (u32, String, Vec<u8>) {
        let u = CString::new(url).unwrap();
        let mut mime: [c_char; 256] = [0; 256];
        let mut buf = vec![0u8; 1 << 20];
        let mut len = 0usize;
        // SAFETY: valid string and buffers of the sizes passed.
        let status = unsafe {
            (self.load)(
                u.as_ptr(),
                kind,
                mime.as_mut_ptr(),
                mime.len(),
                buf.as_mut_ptr(),
                buf.len(),
                &mut len,
            )
        };
        buf.truncate(len);
        // SAFETY: the host wrote a NUL-terminated string into mime.
        let m = unsafe { CStr::from_ptr(mime.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        (status, m, buf)
    }

    fn map(&self, url: &str) -> Option<String> {
        let u = CString::new(url).unwrap();
        let mut out: [c_char; 512] = [0; 512];
        // SAFETY: valid string and buffer.
        let n = unsafe { (self.map)(u.as_ptr(), out.as_mut_ptr(), out.len()) };
        // SAFETY: on success the host wrote a NUL-terminated string.
        (n >= 0).then(|| {
            unsafe { CStr::from_ptr(out.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        })
    }
}

#[test]
fn every_view_serves_fed_addresses_through_the_library() {
    let Some(so) = mock_lib() else {
        eprintln!("no C compiler: the loader is not run against a library here");
        return;
    };
    let lib = ffi::load(Some(&so)).unwrap();
    assert!(lib.set_scheme.is_some(), "the 1.2 symbol is found");
    assert!(webengine::fed::use_resolver(Resolver::new(
        Box::new(Captured),
        federate::ROOT_KEY
    )));
    let hooks = Hooks::open(&so);
    assert_eq!(
        hooks.load("fed://home.fed/").0,
        0,
        "no handler before a view"
    );

    // A plain app: no flag, no setting; making the view registers the handler.
    let dir = std::env::temp_dir().join(format!("wol-mock-app-{}-{}", 12, std::process::id()));
    let policy = Policy {
        origins: vec![format!("file://{}/", dir.display())],
        ..Policy::default()
    };
    let app = App::new(
        "org.wolos.webapp.fed",
        std::env::temp_dir().join(format!("wol-mock-profile-{}-{}", 12, std::process::id())),
    );
    let target = Target::Headless {
        width: 10,
        height: 10,
    };
    let v = WebView::new(
        &policy,
        app,
        target,
        &format!("file://{}/index.html", dir.display()),
    )
    .unwrap();

    let (status, mime, body) = hooks.load("fed://home.fed/");
    assert_eq!((status, mime.as_str()), (200, "text/html; charset=utf-8"));
    assert_eq!(body, INDEX, "the verified block, byte for byte");
    let (status, _, body) = hooks.load_as("fed://home.fed/", ffi::LOAD_RESOURCE);
    assert_eq!(
        (status, body.as_slice()),
        (200, INDEX),
        "a subresource under its cap"
    );
    let (status, _, body) = hooks.load("fed://nobody.fed/");
    assert_eq!(status, 404);
    assert!(
        String::from_utf8_lossy(&body).contains("does not exist"),
        "an honest page"
    );
    let (status, _, body) = hooks.load("fed://home.fed/missing.css");
    assert_eq!(status, 404, "{}", String::from_utf8_lossy(&body));

    assert_eq!(
        hooks.map("http://home.fed/a?b=1").as_deref(),
        Some("fed://home.fed/a?b=1")
    );
    assert_eq!(
        hooks.map("https://lobo.pagina").as_deref(),
        Some("fed://lobo.pagina")
    );
    assert_eq!(
        hooks.map("https://example.com/"),
        None,
        "the web stays the web"
    );
    drop(v);
}
