// SPDX-License-Identifier: BSD-2-Clause
//! The web page's screen, natively: the libwol sample window (`view::sample`, the tree behind the
//! shell's `app-window-parts` golden) raised into wolkit, laid out with the kit look and the
//! shell's tokens, drawn by the CPU reference and by wgpu.
//!
//! * wgpu against the CPU reference: the GPU golden tolerance (`Diff::within_tolerance`).
//! * against the GTK golden of the same tree (`crates/shell/tests/golden/app-window-parts-*.png`):
//!   a different toolkit and text rasteriser draw it, so the bound is loose and stated here:
//!   at most [`GTK_SHARE`] of pixels may differ by more than [`GTK_DELTA`] in any channel.
//!
//! Fonts: Inter (the WolOS UI face) from `WOLKIT_PAGE_FONTS` or `/usr/share/fonts/opentype/inter`;
//! without it the tests pass with a note, unless `WOLKIT_GPU_REQUIRED=1` (the forge lane), where a
//! missing face fails.

#[path = "support/png.rs"]
mod png;

use std::path::PathBuf;

use wolkit::window::{POINTER_DOWN, POINTER_MOVE, POINTER_UP, PointerInput};
use wolkit_render::cpu::CpuRenderer;
use wolkit_render::page::Page;
use wolkit_render::wgpu_backend::WgpuRenderer;
use wolkit_render::{Image, Renderer, diff, view};

/// The loose bound against GTK: share of pixels...
pub const GTK_SHARE: f64 = 0.04;
/// ...that differ by more than this in some channel.
pub const GTK_DELTA: u8 = 48;

fn fonts() -> Option<Vec<Vec<u8>>> {
    let dir = std::env::var_os("WOLKIT_PAGE_FONTS").map_or_else(
        || PathBuf::from("/usr/share/fonts/opentype/inter"),
        PathBuf::from,
    );
    let mut out = Vec::new();
    for f in ["Inter-Regular.otf", "Inter-SemiBold.otf"] {
        match std::fs::read(dir.join(f)) {
            Ok(b) => out.push(b),
            Err(_) => {
                assert!(
                    std::env::var_os("WOLKIT_GPU_REQUIRED").is_none(),
                    "WOLKIT_GPU_REQUIRED is set but {f} is missing in {}",
                    dir.display()
                );
                println!("note: no {f} in {}; page tests skipped", dir.display());
                return None;
            }
        }
    }
    Some(out)
}

fn page(dark: bool) -> Option<Page> {
    let mut p = Page::new(420, 380, 1.0, dark, fonts()?).expect("page");
    p.mount(&view::sample()).expect("mount");
    Some(p)
}

fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("page");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn save(name: &str, img: &Image) {
    let _ = std::fs::write(
        out_dir().join(name),
        png::encode(img.width, img.height, &img.to_straight()),
    );
}

fn gtk_golden(dark: bool) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../shell/tests/golden/app-window-parts-{}.png",
        if dark { "dark" } else { "light" }
    ));
    let file = std::fs::File::open(path).ok()?;
    let mut dec = ::png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(::png::Transformations::EXPAND | ::png::Transformations::ALPHA);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    (info.width == 420 && info.height == 380 && info.color_type == ::png::ColorType::Rgba)
        .then_some(buf)
}

#[test]
fn the_page_draws_the_same_on_wgpu_as_on_the_cpu() {
    let mut r = match WgpuRenderer::new_native(std::env::var_os("WOLKIT_GPU_HARDWARE").is_some()) {
        Ok(r) => r,
        Err(e) => {
            assert!(std::env::var_os("WOLKIT_GPU_REQUIRED").is_none(), "{e}");
            return;
        }
    };
    for (dark, tag) in [(false, "light"), (true, "dark")] {
        let Some(mut p) = page(dark) else {
            return;
        };
        let want = p.draw(|f| CpuRenderer.render_image(f)).unwrap();
        save(&format!("app-window-parts-{tag}-cpu.png"), &want);
        let got = p.draw(|f| r.render_image(f)).unwrap();
        save(&format!("app-window-parts-{tag}-wgpu.png"), &got);
        let d = diff(&got.data, &want.data);
        println!("page {tag}, wgpu vs cpu: {d:?}");
        assert!(d.within_tolerance(), "{tag}: {d:?}");
    }
}

#[test]
fn the_page_is_close_to_the_gtk_golden_of_the_same_tree() {
    for dark in [false, true] {
        let Some(mut p) = page(dark) else {
            return;
        };
        let Some(gtk) = gtk_golden(dark) else {
            println!("note: no GTK golden (crates/shell submodule not checked out)");
            return;
        };
        let img = p.draw(|f| CpuRenderer.render_image(f)).unwrap();
        save(
            &format!(
                "app-window-parts-{}-wolkit.png",
                if dark { "dark" } else { "light" }
            ),
            &img,
        );
        let ours = img.to_straight();
        let mut far = 0usize;
        for (a, b) in ours.as_chunks::<4>().0.iter().zip(gtk.as_chunks::<4>().0) {
            if a.iter().zip(b).any(|(x, y)| x.abs_diff(*y) > GTK_DELTA) {
                far += 1;
            }
        }
        let share = far as f64 / (420.0 * 380.0);
        println!(
            "page vs GTK golden ({}): {far} of 159600 pixels ({:.2}%) differ by more than {GTK_DELTA}",
            if dark { "dark" } else { "light" },
            share * 100.0
        );
        assert!(
            share <= GTK_SHARE,
            "{:.2}% of pixels far from the GTK golden",
            share * 100.0
        );
    }
}

fn center_of(p: &mut Page, node: u32) -> (f32, f32) {
    let r = p.node_rect(node).expect("a laid-out part");
    (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

#[test]
fn a_click_on_the_switch_flips_it_and_reports_the_toggle() {
    let Some(mut p) = page(false) else {
        return;
    };
    let before = p.draw(|f| CpuRenderer.render_image(f)).unwrap();
    let (x, y) = center_of(&mut p, 2);
    let at = |kind| PointerInput {
        kind,
        x,
        y,
        button: wolkit::input::BUTTON_LEFT,
        mods: 0,
    };
    p.pointer(at(POINTER_MOVE));
    p.pointer(at(POINTER_DOWN));
    let ev = p.pointer(at(POINTER_UP));
    assert!(
        ev.iter()
            .any(|e| e.kind == wolkit::event::kind::TOGGLE && e.node == 2),
        "{ev:?}"
    );
    let st = wolkit::scene::SceneRead::state(&p.core.scene, wolkit::scene::NodeId(2).slot());
    assert_eq!(
        st & wolkit::scene::state::CHECKED,
        0,
        "the switch was on and is now off"
    );
    p.advance(1_000_000_000);
    let after = p.draw(|f| CpuRenderer.render_image(f)).unwrap();
    save("app-window-parts-light-toggled.png", &after);
    assert!(
        diff(&before.data, &after.data).over1 > 50,
        "the switch looks the same"
    );
}

#[test]
fn the_named_icon_is_drawn_in_the_text_colour() {
    // `weather-clear-symbolic` (the sun) sits left of "Sunny": a named icon is held in memory
    // (the web build has no icon folder) and raised as `icon:<name>`; it must be on the first frame.
    for dark in [false, true] {
        let Some(mut p) = page(dark) else {
            return;
        };
        let img = p.draw(|f| CpuRenderer.render_image(f)).unwrap();
        // The icon's cell (the row's left end, 32 px icon) in the 420 x 380 window.
        let (x, y, w, h) = (12usize, 62usize, 36usize, 36usize);
        let bg = &img.data[(y * 420 + x) * 4..(y * 420 + x) * 4 + 4].to_vec();
        let mut inked = 0;
        for yy in y..y + h {
            for xx in x..x + w {
                let px = &img.data[(yy * 420 + xx) * 4..(yy * 420 + xx) * 4 + 4];
                if px.iter().zip(bg.iter()).any(|(a, b)| a.abs_diff(*b) > 60) {
                    inked += 1;
                }
            }
        }
        assert!(
            inked > 60,
            "the icon is missing ({inked} pixels differ from the page, dark {dark})"
        );
    }
}
