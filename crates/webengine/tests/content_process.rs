// SPDX-License-Identifier: BSD-2-Clause
//! The process model with the engine: the page runs in one content process, a child of the host
//! started as `<host> --content-process TOKEN` (so it stays in the host's scope), and the page still
//! loads and its script runs. The engine starts one content process per registrable domain of the
//! app's frames (a `file:` document counts as its own), so a page without frames has exactly one.
//! `harness = false`: this binary is that host, and its child re-enters `main` with the
//! token. Needs the library as `headless_golden.rs` does.
use std::path::Path;
use std::time::Duration;
use webengine::{App, Policy, Target, WebView};

fn children_with_token() -> usize {
    let me = std::process::id().to_string();
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return 0;
    };
    rd.flatten()
        .filter(|e| {
            let p = e.path();
            let stat = std::fs::read_to_string(p.join("stat")).unwrap_or_default();
            // pid (comm) state ppid ...
            let ppid = stat
                .rsplit_once(')')
                .and_then(|(_, r)| r.split_whitespace().nth(1));
            let cmd = std::fs::read(p.join("cmdline")).unwrap_or_default();
            ppid == Some(me.as_str())
                && cmd
                    .split(|&b| b == 0)
                    .any(|a| a == webengine::process::CONTENT_FLAG.as_bytes())
        })
        .count()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // The child inherits the environment (the engine's own sandbox is off, src/view.rs).
    let path = std::env::var_os(webengine::ffi::LIB_ENV).map(std::path::PathBuf::from);
    let loaded = webengine::ffi::load(path.as_deref());
    if let Some(token) = webengine::process::content_token(&args) {
        webengine::ffi::run_content_process(token).expect("the host loaded the library");
        return;
    }
    if let Err(e) = loaded {
        if std::env::var_os("WOL_WEBENGINE_REQUIRED").is_some() {
            panic!("{e}");
        }
        eprintln!("content_process: skipped: {e}");
        return;
    }
    let pages = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pages");
    let profile = std::env::temp_dir().join(format!("webengine-mp-{}", std::process::id()));
    let policy = Policy {
        origins: vec![format!("file://{}/", pages.display())],
        ..Policy::default()
    };
    let app = App::new("org.wolos.webapp.mp", profile.clone());
    assert!(app.content_process, "the default is a content process");
    let url = format!("file://{}/plain.html", pages.display());
    let view = WebView::new(
        &policy,
        app,
        Target::Headless {
            width: 320,
            height: 240,
        },
        &url,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    view.wait_loaded(Duration::from_secs(60)).unwrap();
    let kids = children_with_token();
    assert_eq!(kids, 1, "one content process per app");
    let img = view.screenshot(Duration::from_secs(60)).unwrap();
    assert_eq!((img.width, img.height), (320, 240));
    // The script ran in the content process: orange, not the stylesheet's blue.
    let c = &img.rgba[(120 * 320 + 160) * 4..][..3];
    assert!(c[0] > 0xe0 && c[2] < 0x50, "page colour {c:?}");
    drop(view);
    let _ = std::fs::remove_dir_all(&profile);
    println!("content_process: ok (1 content process, page loaded and painted)");
}
