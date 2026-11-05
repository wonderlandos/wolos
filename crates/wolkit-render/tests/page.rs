// SPDX-License-Identifier: BSD-2-Clause
//! The web page's screen, natively: the libwol sample window (`view::sample`, the tree behind the
//! shell's `app-window-parts` golden) raised into wolkit, laid out with the kit look and the
//! shell's tokens, drawn by the CPU reference and by wgpu.
//!
//! * against the GTK golden of the same tree (`crates/shell/tests/golden/app-window-parts-*.png`,
//!   `-w480/-w900/-w1280`, `settings-look-w*`, the theme flip `app-window-parts-flip-*`): zero
//!   differing pixels. Kit app windows draw through libWolkit by default (`crates/shell`
//!   `ui::app_wolkit`), so the shell's golden is this page's CPU frame, byte for byte.
//! * wgpu, the exact path (the CPU frame written into the GPU target, what the web page shows by
//!   default): zero differing pixels.
//! * wgpu, the GPU rasteriser (`?raster=gpu` on the web page): the GPU golden tolerance
//!   (`Diff::within_tolerance`); the blend unit rounds differently from the CPU on some pixels.
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

/// The page sizes of the shell's golden cases (`golden_cases/app_view.rs`): the 420 x 380 sample
/// window, and the responsive widths at 600 high.
pub const WIDTHS: [u32; 3] = [480, 900, 1280];
pub const WIDTH_HEIGHT: u32 = 600;

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
    page_of(&view::sample(), 420, 380, dark)
}

fn page_of(tree: &shared::view::Node, w: u32, h: u32, dark: bool) -> Option<Page> {
    let mut p = Page::new(w, h, 1.0, dark, fonts()?).expect("page");
    p.mount(tree).expect("mount");
    Some(p)
}

fn tag(dark: bool) -> &'static str {
    if dark { "dark" } else { "light" }
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

/// A shell golden (`crates/shell/tests/golden/<name>.png`), RGBA, when it has the size.
fn golden_png(name: &str, w: u32, h: u32) -> Option<Vec<u8>> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../shell/tests/golden/{name}.png"));
    let file = std::fs::File::open(path).ok()?;
    let mut dec = ::png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(::png::Transformations::EXPAND | ::png::Transformations::ALPHA);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    (info.width == w && info.height == h && info.color_type == ::png::ColorType::Rgba)
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

/// Pixels that differ at all between two RGBA buffers (sizes equal).
fn differing(a: &[u8], b: &[u8]) -> usize {
    diff(a, b).any
}

#[test]
fn the_page_is_the_gtk_golden_of_the_same_tree() {
    let mut table = Vec::new();
    let cases = all_cases();
    for (name, tree, w, h) in &cases {
        for dark in [false, true] {
            let Some(mut p) = page_of(tree, *w, *h, dark) else {
                return;
            };
            let t = std::time::Instant::now();
            let img = p.draw(|f| CpuRenderer.render_image(f)).unwrap();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let full = format!("{name}-{}", tag(dark));
            save(&format!("{full}-cpu.png"), &img);
            let Some(gtk) = golden_png(&full, *w, *h) else {
                assert!(
                    std::env::var_os("WOLKIT_GPU_REQUIRED").is_none(),
                    "no GTK golden {full} of {w}x{h}"
                );
                println!("note: no GTK golden {full} (crates/shell submodule not checked out)");
                continue;
            };
            let n = differing(&img.to_straight(), &gtk);
            println!(
                "GTK golden {full} vs wolkit CPU: {n} of {} pixels differ (CPU frame {ms:.1} ms)",
                w * h
            );
            table.push((full, n));
        }
    }
    let bad: Vec<_> = table.iter().filter(|(_, n)| *n != 0).collect();
    assert!(bad.is_empty(), "differing pixels: {bad:?}");
}

/// The component cases: (name, tree, width, height).
fn component_cases() -> Vec<(String, shared::view::Node, u32, u32)> {
    let v: serde_json::Value =
        serde_json::from_str(view::COMPONENTS_JSON).expect("components.json");
    let (w, h) = (
        v["width"].as_u64().unwrap_or(360) as u32,
        v["height"].as_u64().unwrap_or(200) as u32,
    );
    v["cases"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            (
                format!("kit-view-{}", c["name"].as_str().unwrap_or("?")),
                serde_json::from_value(c["tree"].clone()).expect("a view tree"),
                w,
                h,
            )
        })
        .collect()
}

/// Every case the shell draws through libWolkit: the sample window, the widths, the components.
fn all_cases() -> Vec<(String, shared::view::Node, u32, u32)> {
    let mut cases: Vec<(String, shared::view::Node, u32, u32)> =
        vec![("app-window-parts".into(), view::sample(), 420, 380)];
    for w in WIDTHS {
        cases.push((
            format!("app-window-parts-w{w}"),
            view::sample(),
            w,
            WIDTH_HEIGHT,
        ));
        cases.push((
            format!("settings-look-w{w}"),
            view::settings_look(),
            w,
            WIDTH_HEIGHT,
        ));
    }
    cases.extend(component_cases());
    cases
}

#[test]
fn the_exact_wgpu_path_is_the_cpu_frame() {
    let mut r = match WgpuRenderer::new_native(std::env::var_os("WOLKIT_GPU_HARDWARE").is_some()) {
        Ok(r) => r,
        Err(e) => {
            assert!(std::env::var_os("WOLKIT_GPU_REQUIRED").is_none(), "{e}");
            return;
        }
    };
    let cases = all_cases();
    for (name, tree, w, h) in &cases {
        for dark in [false, true] {
            let Some(mut p) = page_of(tree, *w, *h, dark) else {
                return;
            };
            let want = p.draw(|f| CpuRenderer.render_image(f)).unwrap();
            let got = r.upload_image(&want).unwrap();
            let full = format!("{name}-{}", tag(dark));
            save(&format!("{full}-wgpu.png"), &got);
            let d = diff(&got.data, &want.data);
            println!(
                "wgpu exact {full} vs CPU: {} of {} pixels differ",
                d.any, d.pixels
            );
            assert_eq!(d.any, 0, "{full}: {d:?}");
            let raster = p.draw(|f| r.render_image(f)).unwrap();
            let g = diff(&raster.data, &want.data);
            println!(
                "wgpu raster {full} vs CPU: {} differ, {} by more than 1, worst {} ({})",
                g.any,
                g.over1,
                g.worst,
                r.name()
            );
            assert!(g.within_tolerance(), "{full}: {g:?}");
        }
    }
}

#[test]
fn a_theme_flip_redraws_the_page_as_the_other_variant() {
    for dark in [false, true] {
        // Opened in the other variant, then the system theme flips to this one.
        let Some(mut p) = page(!dark) else {
            return;
        };
        let _ = p.draw(|f| CpuRenderer.render_image(f)).unwrap();
        assert!(p.set_theme(dark, Page::default_accent(dark)));
        assert!(
            !p.set_theme(dark, Page::default_accent(dark)),
            "no change the second time"
        );
        let flipped = p.draw(|f| CpuRenderer.render_image(f)).unwrap();
        let Some(mut fresh) = page(dark) else {
            return;
        };
        let want = fresh.draw(|f| CpuRenderer.render_image(f)).unwrap();
        let d = diff(&flipped.data, &want.data);
        println!(
            "theme flip to {}: {} pixels differ from a page opened {0}",
            tag(dark),
            d.any
        );
        assert_eq!(d.any, 0, "{d:?}");
        let full = format!("app-window-parts-flip-{}", tag(dark));
        if let Some(gtk) = golden_png(&full, 420, 380) {
            let n = differing(&flipped.to_straight(), &gtk);
            println!("GTK golden {full} vs wolkit CPU flipped: {n} pixels differ");
            assert_eq!(n, 0, "{full}");
        }
    }
}

#[test]
fn the_page_follows_the_system_theme_file_like_native_apps() {
    // The active theme file (`$XDG_CONFIG_HOME/wolos/theme`), read through the native apps'
    // channel: wol-dark draws the dark page, wol-light the light one.
    let home = std::env::temp_dir().join(format!("wolkit-theme-{}", std::process::id()));
    std::fs::create_dir_all(home.join("wolos")).unwrap();
    let Some(mut p) = page(false) else {
        return;
    };
    for (id, dark) in [("wol-dark", true), ("wol-light", false)] {
        std::fs::write(home.join("wolos/theme"), id).unwrap();
        p.follow_system_theme(&home);
        let got = p.draw(|f| CpuRenderer.render_image(f)).unwrap();
        let mut fresh = page(dark).unwrap();
        let want = fresh.draw(|f| CpuRenderer.render_image(f)).unwrap();
        assert_eq!(diff(&got.data, &want.data).any, 0, "{id}");
    }
    let _ = std::fs::remove_dir_all(&home);
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

/// A click on the part scene node `node`: the events the page reports.
fn click(p: &mut Page, node: u32) -> Vec<wolkit_render::page::PageEvent> {
    let (x, y) = center_of(p, node);
    let at = |kind| PointerInput {
        kind,
        x,
        y,
        button: wolkit::input::BUTTON_LEFT,
        mods: 0,
    };
    p.pointer(at(POINTER_MOVE));
    p.pointer(at(POINTER_DOWN));
    p.pointer(at(POINTER_UP))
}

/// The scene node of the part that asks for `pick` when clicked.
fn node_of(p: &Page, pick: &view::Pick) -> u32 {
    (view::FIRST_STRUCT..view::FIRST_STRUCT + 400)
        .find(|n| p.pick_for(*n) == Some(pick))
        .unwrap_or_else(|| panic!("no part asks for {pick:?}"))
}

#[test]
fn a_click_on_a_design_system_part_asks_the_app_for_the_next_state() {
    use shared::view::{Choice, Node};
    let ch = |k: &str, l: &str| Choice {
        key: k.into(),
        label: l.into(),
    };
    let tree = Node::Stack {
        spacing: 8,
        children: vec![
            Node::Checkbox {
                id: 7,
                label: "Show hidden files".into(),
                checked: false,
                mixed: false,
            },
            Node::RadioGroup {
                id: 8,
                options: vec![ch("a", "Automatic"), ch("b", "Light")],
                selected: "a".into(),
            },
            Node::Tabs {
                id: 9,
                tabs: vec![ch("o", "Overview"), ch("d", "Details")],
                selected: "o".into(),
                children: vec![
                    Node::Label {
                        text: "One".into(),
                        role: shared::view::Role::Body,
                    },
                    Node::Label {
                        text: "Two".into(),
                        role: shared::view::Role::Body,
                    },
                ],
            },
            Node::Disclosure {
                id: 10,
                title: "Advanced".into(),
                open: false,
                children: vec![],
            },
        ],
    };
    let Some(mut p) = page_of(&tree, 420, 380, false) else {
        return;
    };
    for (pick, id) in [
        (view::Pick::Toggled { id: 7, on: true }, 7),
        (
            view::Pick::Picked {
                id: 8,
                key: "b".into(),
            },
            8,
        ),
        (
            view::Pick::Picked {
                id: 9,
                key: "d".into(),
            },
            9,
        ),
        (view::Pick::Toggled { id: 10, on: true }, 10),
    ] {
        let node = node_of(&p, &pick);
        let ev = click(&mut p, node);
        assert!(
            ev.iter()
                .any(|e| e.kind == wolkit::event::kind::CLICK && e.node == node),
            "{id}: {ev:?}"
        );
    }
}

#[test]
fn tabs_show_the_picked_tabs_child_only() {
    use shared::view::{Choice, Node, Role};
    // Like every page test: without the face the test passes with a note (fonts() says so).
    if fonts().is_none() {
        return;
    }
    let tabs = |selected: &str| Node::Tabs {
        id: 1,
        tabs: vec![
            Choice {
                key: "o".into(),
                label: "Overview".into(),
            },
            Choice {
                key: "d".into(),
                label: "Details".into(),
            },
        ],
        selected: selected.into(),
        children: ["One", "Two"]
            .map(|t| Node::Label {
                text: t.into(),
                role: Role::Body,
            })
            .to_vec(),
    };
    let draw = |sel: &str| {
        let mut p = page_of(&tabs(sel), 360, 200, false).expect("fonts");
        p.draw(|f| CpuRenderer.render_image(f)).unwrap()
    };
    let (o, d) = (draw("o"), draw("d"));
    assert!(
        diff(&o.data, &d.data).any > 200,
        "the picked tab did not change the panel"
    );
    assert_eq!(
        diff(&o.data, &draw("o").data).any,
        0,
        "drawing is deterministic"
    );
}
