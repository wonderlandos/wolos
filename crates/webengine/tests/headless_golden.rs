// SPDX-License-Identifier: BSD-2-Clause
//! The engine end to end, headless: a local page under a policy that allows only its folder renders
//! to the golden PNG (`tests/golden/golden.png`; `WOL_BLESS=1` rewrites it, review the diff), its
//! script ran, and the gate refused the frame and the image from another origin; a click on a link
//! to another origin is refused too and leaves the page as it was (the host offers it in the
//! browser).
//!
//! Needs `libwolwebengine.so.1` (the webengine-wolos package, or `WOL_WEBENGINE_LIB=<path>`). Without
//! it the test says so and passes, unless `WOL_WEBENGINE_REQUIRED=1` (the package's check() and
//! the forge run set it), where a missing library fails.
#[path = "../../wolkit/tests/golden/png.rs"]
mod png;

use std::path::Path;
use std::time::Duration;
use webengine::input::Input;
use webengine::{App, Policy, Target, WebView};

const W: u32 = 320;
const H: u32 = 240;

/// Pixels may differ by this much per channel (rasteriser and font hinting across Mesa and
/// FreeType builds) ...
const CHANNEL: u8 = 24;
/// ... on at most this share of the image.
const SHARE: f64 = 0.01;

fn engine_or_skip() -> bool {
    let path = std::env::var_os(webengine::ffi::LIB_ENV).map(std::path::PathBuf::from);
    match webengine::ffi::load(path.as_deref()) {
        Ok(_) => true,
        Err(e) if std::env::var_os("WOL_WEBENGINE_REQUIRED").is_some() => panic!("{e}"),
        Err(e) => {
            eprintln!("skipped: {e} (set WOL_WEBENGINE_LIB, or WOL_WEBENGINE_REQUIRED=1 to fail)");
            false
        }
    }
}

#[test]
fn a_local_page_renders_to_the_golden_and_the_gate_holds() {
    if !engine_or_skip() {
        return;
    }
    let pages = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pages");
    let profile = std::env::temp_dir().join(format!("webengine-golden-{}", std::process::id()));
    let policy = Policy {
        origins: vec![format!("file://{}/", pages.display())],
        ..Policy::default()
    };
    let mut app = App::new("org.wolos.webapp.golden", profile.clone());
    app.content_process = false;
    let url = format!("file://{}/golden.html", pages.display());
    let view = WebView::new(
        &policy,
        app,
        Target::Headless {
            width: W,
            height: H,
        },
        &url,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    view.wait_loaded(Duration::from_secs(60)).unwrap();
    let img = view.screenshot(Duration::from_secs(60)).unwrap();
    assert_eq!((img.width, img.height), (W, H));

    // The script ran: its box is orange, not the stylesheet's grey.
    let px = |x: usize, y: usize| {
        let i = (y * W as usize + x) * 4;
        [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]]
    };
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 8);
    assert!(
        near(px(268, 52), [0xf0, 0xa0, 0x30]),
        "script box {:?}",
        px(268, 52)
    );
    assert!(
        near(px(120, 120), [0x3b, 0x6d, 0xf0]),
        "card {:?}",
        px(120, 120)
    );

    // The gate: the other origin's frame document and image were refused as loads (a frame's
    // first load is not a navigation the host is offered), and the page still loaded.
    assert!(
        view.refused_loads() >= 2,
        "{} refused loads",
        view.refused_loads()
    );
    assert_eq!(view.take_refused(), Vec::<String>::new());
    // Storage went to the profile directory only.
    assert!(profile.is_dir());

    // A click on the link to another origin: refused, and the page stays.
    let (x, y) = (60.0, 216.0);
    view.input(&Input::MouseMove { x, y });
    view.input(&Input::MouseButton {
        button: 0,
        pressed: true,
        x,
        y,
    });
    view.input(&Input::MouseButton {
        button: 0,
        pressed: false,
        x,
        y,
    });
    let mut refused = Vec::new();
    view.spin_until(Duration::from_secs(20), |v| {
        refused.extend(v.take_refused());
        !refused.is_empty()
    });
    assert_eq!(refused, ["https://blocked.example/away"]);
    // The same page: every pixel but the link's own box and its focus ring (24,204 80x24).
    let after = view.screenshot(Duration::from_secs(60)).unwrap();
    let w = W as usize;
    let ring = |i: usize| (20..=108).contains(&(i % w)) && (200..=232).contains(&(i / w));
    let same = (0..w * H as usize)
        .filter(|&i| !ring(i))
        .all(|i| after.rgba[i * 4..i * 4 + 4] == img.rgba[i * 4..i * 4 + 4]);
    if !same {
        let out = std::env::temp_dir().join("webengine-golden-after.png");
        std::fs::write(&out, png::encode(W, H, &after.rgba)).unwrap();
        panic!(
            "the page changed after a refused navigation: {}",
            out.display()
        );
    }

    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/golden.png");
    if std::env::var_os("WOL_BLESS").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, png::encode(W, H, &img.rgba)).unwrap();
        eprintln!("blessed {}", golden.display());
    }
    let want = std::fs::read(&golden).expect("golden missing: run once with WOL_BLESS=1");
    let (gw, gh, gpx) = png::decode(&want).unwrap();
    assert_eq!((gw, gh), (W, H));
    let off = img
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(gpx.as_chunks::<4>().0)
        .filter(|(a, b)| {
            a.iter()
                .zip(b.iter())
                .any(|(x, y)| x.abs_diff(*y) > CHANNEL)
        })
        .count();
    let share = off as f64 / f64::from(W * H);
    if share > SHARE {
        let out = std::env::temp_dir().join("webengine-golden-actual.png");
        std::fs::write(&out, png::encode(W, H, &img.rgba)).unwrap();
        panic!(
            "{off} pixels differ ({share:.4}); actual at {}",
            out.display()
        );
    }
    drop(view);
    let _ = std::fs::remove_dir_all(&profile);
}
