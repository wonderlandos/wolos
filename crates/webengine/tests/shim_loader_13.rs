// SPDX-License-Identifier: BSD-2-Clause
//! The loader and the view against the C boundary, with a stand-in library (`tests/mock_engine.c`,
//! built with `cc`; skipped where there is none) of the 1.3 build, which has `wwe_view_present`: a
//! window target shows nothing until the frame the engine painted is presented, so the host's
//! `spin` must present each new frame, once (the real engine's wl_surface stayed empty without
//! it).
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use webengine::{App, Policy, Target, WebView, ffi};

/// The stand-in library, or None without a C compiler.
fn mock_lib() -> Option<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mock_engine.c");
    let out = std::env::temp_dir().join(format!("wol-mock-engine-{}-{}", 13, std::process::id()));
    std::fs::create_dir_all(&out).ok()?;
    let so = out.join("libwolwebengine.so.1");
    let mut cc = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()));
    cc.args(["-shared", "-fPIC", "-DPRESENT", "-o"])
        .arg(&so)
        .arg(&src);
    cc.status().ok()?.success().then_some(so)
}

/// What the stand-in wrote of the presents it got: "<presents> frames <frames painted>".
fn presents(profile: &std::path::Path) -> String {
    std::fs::read_to_string(profile.join("presented"))
        .unwrap_or_default()
        .trim()
        .to_owned()
}

#[test]
fn spin_presents_each_new_frame_once() {
    let Some(so) = mock_lib() else {
        eprintln!("no C compiler: the loader is not run against a library here");
        return;
    };
    let lib = ffi::load(Some(&so)).unwrap();
    assert!(lib.view_present.is_some(), "the 1.3 library is seen");
    let id = std::process::id();
    let dir = std::env::temp_dir().join(format!("wol-mock-app-13-{id}"));
    let profile = std::env::temp_dir().join(format!("wol-mock-profile-13-{id}"));
    let _ = std::fs::remove_dir_all(&profile);
    let policy = Policy {
        origins: vec![format!("file://{}/", dir.display())],
        ..Policy::default()
    };
    let v = WebView::new(
        &policy,
        App::new("org.wolos.webapp.present", profile.clone()),
        Target::Headless {
            width: 100,
            height: 100,
        },
        &format!("file://{}/index.html", dir.display()),
    )
    .unwrap();
    // Nothing painted yet: nothing to show.
    assert_eq!(presents(&profile), "");
    v.spin();
    assert_eq!(presents(&profile), "1 frames 1", "the first frame is shown");
    // Spinning with no new frame shows nothing again.
    v.spin();
    v.spin();
    assert_eq!(presents(&profile), "1 frames 1", "once per frame");
    // A resize paints a new frame: shown by the next spin.
    v.resize(200, 120, 1.0);
    assert_eq!(
        presents(&profile),
        "1 frames 1",
        "not before the engine paints"
    );
    assert!(v.spin_until(Duration::from_secs(2), |v| v.state().frames == 2));
    assert_eq!(presents(&profile), "2 frames 2");
    drop(v);
    let _ = std::fs::remove_dir_all(&profile);
}
