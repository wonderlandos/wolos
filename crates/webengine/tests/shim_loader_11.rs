// SPDX-License-Identifier: BSD-2-Clause
//! The loader and the view against the C boundary, with a stand-in library (`tests/mock_engine.c`,
//! built with `cc`; skipped where there is none) of the 1.1 build: the page's messages reach the host's callback and the host's reach the page.
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use webengine::Channel;
use webengine::{App, Policy, Target, WebView, ffi};

const CHANNEL: bool = true;

/// The stand-in library, or None without a C compiler.
fn mock_lib() -> Option<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_engine.c");
    let out = std::env::temp_dir().join(format!("wol-mock-engine-{}-{}", 11, std::process::id()));
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
    let dir = std::env::temp_dir().join(format!("wol-mock-app-{}-{}", 11, std::process::id()));
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
        std::env::temp_dir().join(format!("wol-mock-profile-{}-{}", 11, std::process::id())),
    )
}

#[test]
fn the_page_and_the_host_talk_through_the_libraries_channel() {
    let Some(so) = mock_lib() else {
        eprintln!("no C compiler: the loader is not run against a library here");
        return;
    };
    let lib = ffi::load(Some(&so)).unwrap();
    assert!(lib.set_host_channel.is_some() && lib.post_message.is_some());
    let got: Rc<RefCell<Vec<String>>> = Rc::default();
    let sink = got.clone();
    let mut a = app("new");
    a.channel = Some(Channel {
        origin: "file:///tmp/x/".into(),
        init: r#"{"app":{}}"#.into(),
        on_message: Box::new(move |t| sink.borrow_mut().push(t.to_owned())),
    });
    let v = view(a).unwrap();
    assert!(v.has_channel());
    // The mock engine answers a host message by posting it back as the page would.
    assert!(v.post_message("hi"));
    assert!(v.post_message("héllo"));
    assert_eq!(*got.borrow(), ["echo:hi", "echo:héllo"]);
    assert!(
        !v.post_message(&"x".repeat(webengine::bridge::MAX_MESSAGE + 1)),
        "over the cap"
    );
    drop(v);

    // The library refuses an origin that is not a folder of files: the view is not made.
    let mut a = app("bad");
    a.channel = Some(Channel {
        origin: "https://example.org/".into(),
        init: "{}".into(),
        on_message: Box::new(|_| {}),
    });
    let e = view(a).err().expect("refused");
    assert!(e.to_string().contains("bad origin"), "{e}");
}
