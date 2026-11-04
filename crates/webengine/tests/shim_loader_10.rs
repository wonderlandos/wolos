// SPDX-License-Identifier: BSD-2-Clause
//! The loader and the view against the C boundary, with a stand-in library (`tests/mock_engine.c`,
//! built with `cc`; skipped where there is none) of the 1.0 build, without the WWE_1.1 host channel: the app opens and runs in development mode.
use std::path::PathBuf;
use std::process::Command;
use webengine::{App, Policy, Target, WebView, ffi};

const CHANNEL: bool = false;

/// The stand-in library, or None without a C compiler.
fn mock_lib() -> Option<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_engine.c");
    let out = std::env::temp_dir().join(format!("wol-mock-engine-{}-{}", 10, std::process::id()));
    std::fs::create_dir_all(&out).ok()?;
    let so = out.join("libwolwebengine.so.1");
    let mut cc = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()));
    cc.args(["-shared", "-fPIC", "-o"]).arg(&so).arg(&src);
    if CHANNEL {
        cc.arg("-DCHANNEL");
    }
    cc.status().ok()?.success().then_some(so)
}

fn view(app: App) -> Result<WebView, webengine::Error> {
    let dir = std::env::temp_dir().join(format!("wol-mock-app-{}-{}", 10, std::process::id()));
    let policy = Policy {
        origins: vec![format!("file://{}/", dir.display())],
        ..Policy::default()
    };
    WebView::new(
        &policy,
        app,
        Target::Headless {
            width: 100,
            height: 100,
        },
        &format!("file://{}/index.html", dir.display()),
    )
}

fn app(name: &str) -> App {
    App::new(
        &format!("org.wolos.webapp.{name}"),
        std::env::temp_dir().join(format!("wol-mock-profile-{}-{}", 10, std::process::id())),
    )
}

#[test]
fn a_library_without_the_channel_is_loaded_and_the_app_runs_without_one() {
    let Some(so) = mock_lib() else {
        eprintln!("no C compiler: the loader is not run against a library here");
        return;
    };
    let lib = ffi::load(Some(&so)).unwrap();
    assert!(lib.set_host_channel.is_none() && lib.post_message.is_none());
    let mut a = app("old");
    a.channel = Some(webengine::Channel {
        origin: "file:///tmp/x/".into(),
        init: "{}".into(),
        on_message: Box::new(|_| panic!("a 1.0 library posts nothing")),
    });
    let v = view(a).expect("the view opens without the channel");
    assert!(!v.has_channel(), "development mode");
    assert!(!v.post_message("hi"));
}
