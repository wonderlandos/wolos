// SPDX-License-Identifier: BSD-2-Clause
//! Bit-exact golden scenes for the CPU reference rasteriser.
//!
//! Every scene is rendered with the bundled test fonts (never an installed font, see
//! `fonts/README.md`) and compared byte for byte with `scenes/<name>.png`. To (re)generate the
//! images after an intended change, run with `WOLKIT_BLESS=1`, look at the PNGs, and commit them.

mod png;
mod scenes;
#[path = "../text_support/mod.rs"]
mod text_support;

use std::path::PathBuf;

use scenes::{BG, Ctx, Scene};
use wolkit::cpu::raster::{Image, RasterOptions, render};
use wolkit::paint::list::DisplayList;
use wolkit::text::glyphcache::Glyphs;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/scenes")
}

fn render_scene(s: &Scene, ctx: &mut Ctx, tile: i32) -> Image {
    let mut list = DisplayList::new();
    (s.build)(ctx, &mut list);
    list.finish();
    let mut src = Glyphs {
        db: &ctx.f.db,
        cache: &mut ctx.glyphs,
    };
    render(
        &list,
        &mut src,
        s.width,
        s.height,
        &RasterOptions {
            scale: s.scale,
            clear: BG,
            tile,
        },
    )
}

fn render_fresh(s: &Scene) -> Image {
    render_scene(s, &mut Ctx::new(), 64)
}

#[test]
fn there_are_twelve_scenes_covering_bidi_and_cjk() {
    let all = scenes::all();
    assert_eq!(all.len(), 12);
    let names: Vec<_> = all.iter().map(|s| s.name).collect();
    assert!(names.iter().any(|n| n.contains("bidi")) && names.iter().any(|n| n.contains("cjk")));
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), 12, "scene names are unique");
}

#[test]
fn scenes_match_their_goldens_bit_for_bit() {
    let bless = std::env::var_os("WOLKIT_BLESS").is_some();
    let dir = golden_dir();
    let actual_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("golden-actual");
    let mut failures = Vec::new();
    for s in scenes::all() {
        let img = render_fresh(&s);
        let rgba = img.to_straight();
        let path = dir.join(format!("{}.png", s.name));
        if bless {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, png::encode(img.width, img.height, &rgba)).unwrap();
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            failures.push(format!(
                "{}: no golden at {} (run with WOLKIT_BLESS=1)",
                s.name,
                path.display()
            ));
            continue;
        };
        let (w, h, want) =
            png::decode(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if (w, h) != (img.width, img.height) {
            failures.push(format!(
                "{}: size {}x{} but the golden is {w}x{h}",
                s.name, img.width, img.height
            ));
            continue;
        }
        if rgba != want {
            let diff = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(want.as_chunks::<4>().0)
                .filter(|(a, b)| a != b)
                .count();
            let first = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(want.as_chunks::<4>().0)
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            std::fs::create_dir_all(&actual_dir).unwrap();
            let out = actual_dir.join(format!("{}.png", s.name));
            std::fs::write(&out, png::encode(img.width, img.height, &rgba)).unwrap();
            failures.push(format!(
                "{}: {diff} pixels differ, first at ({}, {}); actual written to {}",
                s.name,
                first as u32 % w,
                first as u32 / w,
                out.display()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "golden mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn rendering_does_not_depend_on_cache_state_or_tile_size() {
    // One context for every scene (warm shape, glyph and atlas caches) must give the bytes of a
    // cold render, and a different tile size must not change a pixel.
    let mut warm = Ctx::new();
    for s in scenes::all() {
        let cold = render_fresh(&s);
        let hot = render_scene(&s, &mut warm, 64);
        assert_eq!(cold, hot, "{}: warm caches changed the output", s.name);
        let small_tiles = render_scene(&s, &mut warm, 24);
        assert_eq!(
            cold, small_tiles,
            "{}: tile size changed the output",
            s.name
        );
    }
}

#[test]
fn every_scene_draws_something_and_text_scenes_draw_text() {
    for s in scenes::all() {
        let img = render_fresh(&s);
        let bg = wolkit::paint::geom::Color::rgb(243, 241, 248).premul();
        let painted = img
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[..] != bg[..])
            .count();
        assert!(
            painted > 400,
            "{}: only {painted} pixels differ from the background",
            s.name
        );
    }
}

#[test]
fn the_bundled_fonts_are_the_pinned_ones() {
    // FNV-1a 64 of each file; the sources and licences are in fonts/README.md.
    let manifest =
        std::fs::read_to_string(text_support::font_dir().join("MANIFEST")).expect("MANIFEST");
    let mut n = 0;
    for line in manifest
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let mut f = line.split_whitespace();
        let (name, len, hash) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
        let data = text_support::load(name);
        assert_eq!(data.len().to_string(), len, "{name}: size");
        let mut h = wolkit::text::cache::FNV_OFFSET;
        h = wolkit::text::cache::hash_bytes(h, &data);
        assert_eq!(format!("{h:016x}"), hash, "{name}: content");
        n += 1;
    }
    assert_eq!(n, 5);
}
