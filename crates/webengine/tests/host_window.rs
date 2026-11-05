// SPDX-License-Identifier: BSD-2-Clause
//! The host's window (`webengine::window`, used by `wol-webhost`), without a display: the engine
//! must ask for Wolkit's native Wayland backend, and the keys the compositor sends must reach the
//! page (they arrive as records of the window node, QA-WEB-13). Each test is one of the defects
//! WEBAPP-bench found on `wave3/int`.
use webengine::input::{self, Bridged, Input};
use webengine::window::{SURFACE, native_engine, surface_patch};
use wolkit::abi::ENGINE_HEADLESS;
use wolkit::engine::Engine;
use wolkit::event::HEADER;
use wolkit::window::KeyInput;

/// Every record the engine has queued for the page, bridged.
fn bridged(e: &Engine) -> Vec<Bridged> {
    let mut buf = vec![0u8; 1 << 16];
    let n = e.read_events(&mut buf);
    let mut out = Vec::new();
    let mut at = 0;
    while at + HEADER <= n {
        let len = u32::from_le_bytes(buf[at..at + 4].try_into().unwrap()) as usize;
        out.extend(input::bridge(&buf[at..at + len], SURFACE));
        at += len;
    }
    out
}

#[test]
fn the_host_asks_for_the_native_backend() {
    // Process-wide state, so this is the only test in this file that touches it.
    assert!(!wolkit::window::native::opted_in(), "nothing opted in yet");
    let env = |k: &str| (k == "LANG").then(|| "pt_BR.UTF-8".to_owned());
    let e = native_engine("org.wolos.webapp.test", env).unwrap();
    assert_eq!(
        wolkit::window::native::session().locale,
        "pt_BR.UTF-8",
        "the session the window runs in comes from the environment given"
    );
    assert!(
        wolkit::window::native::opted_in(),
        "without the opt-in create_window fails with \"The WolOS desktop is not running\""
    );
    drop(e);
}

#[test]
fn keys_reach_the_page_surface() {
    let e = Engine::new("org.wolos.webapp.test", ENGINE_HEADLESS).unwrap();
    let win = e.create_window("T", 400, 300, 0).unwrap();
    e.apply(win, &surface_patch()).unwrap();
    e.show(win).unwrap();
    e.dispatch(0).unwrap();
    for (sym, pressed) in [(0xff56, true), (0xff56, false)] {
        e.inject_key(
            win,
            KeyInput {
                keysym: sym,
                pressed,
                mods: 0,
            },
        )
        .unwrap();
    }
    let got = bridged(&e);
    let keys: Vec<_> = got
        .iter()
        .filter_map(|b| match b {
            Bridged::Input(Input::Key { key, pressed, .. }) => Some((key.as_str(), *pressed)),
            _ => None,
        })
        .collect();
    assert_eq!(
        keys,
        [("PageDown", true), ("PageDown", false)],
        "the window node binds KEY and the page takes the keys from it; got {got:?}"
    );
}
