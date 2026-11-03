// SPDX-License-Identifier: BSD-2-Clause
//! Pictures end to end: PNG (made here) and JPEG (fixtures with an EXIF orientation) decoding,
//! the size cap, the 1.1 verbs through the C ABI, `image` nodes lowered with their fit and
//! rounded corners, the placeholder while decoding, and a headless window drawing a picture
//! named by path (it waits for the decode, so the first frame already shows it).

use std::ptr::null_mut;
use std::sync::Arc;
use std::time::Duration;

use wolkit::env::Environment;
use wolkit::ffi::engine::{wolkit_engine, wolkit_engine_create, wolkit_engine_release};
use wolkit::ffi::image::{
    wolkit_image, wolkit_image_create_encoded, wolkit_image_create_file, wolkit_image_get_id,
    wolkit_image_get_size, wolkit_image_get_state, wolkit_image_release,
};
use wolkit::image::{Entry, STATE_FAILED, STATE_READY, decode, settle};
use wolkit::layout::LayoutCache;
use wolkit::paint::list::{DisplayList, Fill, Item};
use wolkit::paint::palette::Palette;
use wolkit::paint::scene::{Lower, build};
use wolkit::scene::patch::{APPEND, Patch, Scratch, Writer};
use wolkit::scene::prop::{DIM_PX, id};
use wolkit::scene::{Journal, Kind, Scene};
use wolkit::style::tokens::Token;
use wolkit::style::{Styles, Table, compile_sources, resolve};
use wolkit::text::measure::TextCache;
use wolkit::text::system::TextSystem;

const WAIT: Duration = Duration::from_secs(20);

fn png_bytes(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(color);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(data).unwrap();
    }
    out
}

fn solid_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let data: Vec<u8> = rgba
        .iter()
        .copied()
        .cycle()
        .take((w * h * 4) as usize)
        .collect();
    png_bytes(w, h, png::ColorType::Rgba, &data)
}

fn fixture(name: &str) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/image")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn near(a: [u8; 4], b: [u8; 4], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

#[test]
fn png_decodes_to_premultiplied_rgba_in_every_colour_type() {
    let p = decode::decode(&solid_png(3, 2, [200, 100, 50, 128]), 0).unwrap();
    assert_eq!((p.width, p.height), (3, 2));
    assert_eq!(p.levels[0].texel(2, 1), [100, 50, 25, 128]);
    let gray = decode::decode(
        &png_bytes(2, 2, png::ColorType::Grayscale, &[9, 9, 9, 9]),
        0,
    )
    .unwrap();
    assert_eq!(gray.levels[0].texel(0, 0), [9, 9, 9, 255]);
    let rgb = decode::decode(&png_bytes(1, 1, png::ColorType::Rgb, &[1, 2, 3]), 0).unwrap();
    assert_eq!(rgb.levels[0].texel(0, 0), [1, 2, 3, 255]);
    let ga = png_bytes(1, 1, png::ColorType::GrayscaleAlpha, &[255, 51]);
    assert_eq!(
        decode::decode(&ga, 0).unwrap().levels[0].texel(0, 0),
        [51, 51, 51, 51]
    );
}

#[test]
fn jpeg_orientation_six_turns_the_picture_a_quarter() {
    // The fixture is 16 x 8, red on the left half and blue on the right, tagged "turn 90
    // degrees clockwise": upright it is 8 x 16 with red on top.
    let b = fixture("orient6.jpg");
    let h = decode::probe(&b).unwrap();
    assert_eq!((h.width, h.height, h.orientation), (8, 16, 6));
    let p = decode::decode(&b, 0).unwrap();
    assert_eq!((p.width, p.height), (8, 16));
    let l = &p.levels[0];
    assert_eq!((l.w, l.h), (8, 16));
    assert!(
        near(l.texel(4, 2), [230, 20, 20, 255], 24),
        "{:?}",
        l.texel(4, 2)
    );
    assert!(
        near(l.texel(4, 13), [20, 20, 230, 255], 24),
        "{:?}",
        l.texel(4, 13)
    );
    let g = decode::decode(&fixture("gray.jpg"), 0).unwrap();
    assert!(near(g.levels[0].texel(3, 3), [128, 128, 128, 255], 3));
}

#[test]
fn a_size_cap_shrinks_the_decoded_picture() {
    let b = solid_png(400, 200, [10, 200, 30, 255]);
    let p = decode::decode(&b, 100).unwrap();
    assert_eq!((p.width, p.height), (400, 200), "the natural size is kept");
    assert_eq!((p.levels[0].w, p.levels[0].h), (100, 50));
    assert_eq!(p.levels[0].texel(50, 25), [10, 200, 30, 255]);
}

#[test]
fn damaged_and_unknown_bytes_are_refused_with_a_sentence() {
    let mut b = solid_png(8, 8, [1, 2, 3, 255]);
    b.truncate(b.len() / 2);
    let e = decode::decode(&b, 0).unwrap_err();
    assert!(e.message.ends_with('.'), "{}", e.message);
    assert!(decode::decode(b"GIF89a....", 0).is_err());
    let mut j = fixture("orient6.jpg");
    j.truncate(200);
    assert!(decode::decode(&j, 0).is_err());
    assert!(Entry::from_encoded(b"not a picture".to_vec(), 0).is_err());
}

/// Deterministic mutations of both fixtures never panic (the fuzz target's corpus on stable).
#[test]
fn mutated_pictures_never_panic_the_decoder() {
    let seeds = [fixture("orient6.jpg"), solid_png(5, 3, [1, 2, 3, 4])];
    let mut x: u32 = 0x1234_5678;
    for seed in &seeds {
        for _ in 0..300 {
            let mut b = seed.clone();
            for _ in 0..4 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let at = (x as usize) % b.len();
                b[at] = (x >> 24) as u8;
            }
            let _ = decode::decode(&b, 64);
        }
    }
    assert!(decode::decode(&seeds[1], 0).is_ok());
}

struct Engine(*mut wolkit_engine);

impl Engine {
    fn headless() -> Engine {
        let id = "org.example.pictures";
        let mut e = null_mut();
        // SAFETY: a valid id and out-pointer.
        let st =
            unsafe { wolkit_engine_create(id.as_ptr().cast(), id.len(), 1, &mut e, null_mut()) };
        assert_eq!(st, 0);
        Engine(e)
    }
}

// Releases the engine reference.
impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: the reference this test owns.
        unsafe { wolkit_engine_release(self.0) };
    }
}

/// A picture made through the C ABI: from bytes, or (with `file`) from a path.
fn create(e: &Engine, bytes: &[u8], file: bool, cap: u32) -> (i32, *mut wolkit_image) {
    let mut out = null_mut();
    let (p, n) = (bytes.as_ptr(), bytes.len());
    // SAFETY: a live engine, a valid buffer and out-pointer.
    let st = unsafe {
        if file {
            wolkit_image_create_file(e.0, p.cast(), n, cap, &mut out, null_mut())
        } else {
            wolkit_image_create_encoded(e.0, p, n, cap, &mut out, null_mut())
        }
    };
    (st, out)
}

/// `(id, state, size)` of a live picture through the C ABI, then releases it.
fn inspect_and_release(i: *mut wolkit_image) -> (u32, u32, Option<(u32, u32)>) {
    let (mut w, mut h) = (0, 0);
    // SAFETY: a live image this test owns, writable outs; released last.
    unsafe {
        let id = wolkit_image_get_id(i);
        let st = wolkit_image_get_state(i);
        let size = (wolkit_image_get_size(i, &mut w, &mut h) == 0).then_some((w, h));
        wolkit_image_release(i);
        (id, st, size)
    }
}

/// The size through the C ABI (known from the header before the decode).
fn size(i: *const wolkit_image) -> Option<(u32, u32)> {
    let (mut w, mut h) = (0, 0);
    // SAFETY: a live image and writable outs.
    (unsafe { wolkit_image_get_size(i, &mut w, &mut h) } == 0).then_some((w, h))
}

#[test]
fn the_new_verbs_create_pictures_from_memory_and_files() {
    let e = Engine::headless();
    let b = solid_png(40, 20, [255, 0, 0, 255]);
    let (st, img) = create(&e, &b, false, 10);
    assert_eq!(st, 0);
    assert_eq!(size(img), Some((40, 20)), "known from the header at once");
    assert!(settle(WAIT));
    let (id, state, _) = {
        let id = inspect_id(img);
        let entry = wolkit::image::lookup(id).unwrap();
        assert_eq!(entry.pixels().unwrap().levels[0].w, 10, "capped");
        drop(entry);
        inspect_and_release(img)
    };
    assert_eq!(state, STATE_READY);
    assert!(wolkit::image::lookup(id).is_none(), "released");

    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wolkit-image");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("red.png");
    std::fs::write(&path, &b).unwrap();
    let (st, f) = create(&e, path.to_str().unwrap().as_bytes(), true, 0);
    assert_eq!(st, 0);
    assert!(settle(WAIT));
    let (_, state, wh) = inspect_and_release(f);
    assert_eq!((state, wh), (STATE_READY, Some((40, 20))));

    let (st, m) = create(&e, b"/nonexistent/wolkit/none.png", true, 0);
    assert_eq!(st, 0, "the file is read on a worker");
    assert!(settle(WAIT));
    let (_, state, wh) = inspect_and_release(m);
    assert_eq!((state, wh), (STATE_FAILED, None));

    assert_eq!(
        create(&e, b"pictures/a.png", true, 0).0,
        1,
        "a relative path is refused"
    );
    assert_eq!(
        create(&e, &b, false, 20_000).0,
        1,
        "a cap over the side limit is refused"
    );
    assert_eq!(create(&e, b"GIF89a", false, 0).0, 1, "not a picture");
}

fn inspect_id(i: *const wolkit_image) -> u32 {
    // SAFETY: a live image.
    unsafe { wolkit_image_get_id(i) }
}

struct Win {
    scene: Scene,
    styles: Styles,
    layout: LayoutCache,
    table: Arc<Table>,
    env: Environment,
    text: TextSystem,
    cache: TextCache,
}

const SHEET: &str = ".round { border-radius: 10px; } .faded { opacity: 0.5; }";

fn px(v: f32) -> [u32; 2] {
    [DIM_PX, v.to_bits()]
}

impl Win {
    fn new(f: impl FnOnce(&mut Writer, &dyn Fn(&str) -> u32)) -> Win {
        let c = compile_sources(&[("t.wolcss", SHEET)])
            .unwrap_or_else(|d| panic!("{}", wolkit::style::render_all(&d)));
        let table = Arc::new(Table::new(c.table));
        let mut w = Writer::new();
        let class = |n: &str| table.table.class_id(n).unwrap();
        f(&mut w, &class);
        let bytes = w.finish(1).to_vec();
        let mut scene = Scene::new();
        let p = Patch::parse(&bytes, &mut Scratch::default()).unwrap();
        scene.apply(&p, &mut Journal::default()).unwrap();
        let mut win = Win {
            scene,
            styles: Styles::default(),
            layout: LayoutCache::new(),
            table,
            env: Environment::default(),
            text: TextSystem::from_db(wolkit::text::fontdb::FontDb::default()),
            cache: TextCache::new(),
        };
        resolve(
            &mut win.scene,
            Some(&win.table),
            &mut win.styles,
            &win.env,
            200,
        );
        wolkit::layout::scene::run_text(
            &mut win.scene,
            &win.styles,
            &mut win.layout,
            (&mut win.text, &mut win.cache),
            200.0,
            200.0,
        );
        win
    }

    fn list(&mut self) -> DisplayList {
        let palette = Palette::for_env(&self.env);
        let mut list = DisplayList::new();
        build(
            &mut Lower {
                scene: &self.scene,
                styles: &self.styles,
                palette: &palette,
                text: &mut self.text,
                cache: &mut self.cache,
            },
            &mut list,
        );
        list
    }
}

fn image_node(w: &mut Writer, class: &dyn Fn(&str) -> u32, image: u32, fit: u32, classes: &[&str]) {
    w.create(1, Kind::Image).insert(0, 1, APPEND);
    w.set_prop(1, id::WIDTH, &px(80.0));
    w.set_prop(1, id::HEIGHT, &px(40.0));
    w.set_prop(1, id::IMAGE_ID, &[image]);
    w.set_prop(1, id::IMAGE_FIT, &[fit]);
    let ids: Vec<u32> = classes.iter().map(|c| class(c)).collect();
    w.set_style(1, &ids);
}

#[test]
fn an_image_node_lowers_to_a_rounded_clip_and_a_covering_picture() {
    let e = Entry::from_rgba(40, 40, 160, &[90; 40 * 40 * 4]).unwrap();
    let mut win = Win::new(|w, c| image_node(w, c, e.id, 1, &["round"]));
    let list = win.list();
    let items = list.chunk_items(&list.chunks[0]);
    let [
        Item::PushClip { radius, .. },
        Item::Image {
            rect, dest, alpha, ..
        },
        Item::PopClip,
    ] = items
    else {
        panic!("{items:?}");
    };
    assert_eq!(*radius, 10.0);
    assert_eq!((rect.w, rect.h), (80.0, 40.0));
    assert_eq!((dest.w, dest.h), (80.0, 80.0), "cover keeps the aspect");
    assert_eq!(dest.y, rect.y - 20.0, "and centres the overflow");
    assert_eq!(*alpha, 255);
}

#[test]
fn a_faded_bare_picture_needs_no_layer_and_contain_letterboxes() {
    let e = Entry::from_rgba(40, 40, 160, &[90; 40 * 40 * 4]).unwrap();
    let mut win = Win::new(|w, c| image_node(w, c, e.id, 0, &["faded"]));
    let list = win.list();
    let items = list.chunk_items(&list.chunks[0]);
    assert!(
        matches!(
            items.first(),
            Some(Item::PushOpacity {
                alpha: 128,
                overlap: false
            })
        ),
        "{items:?}"
    );
    let Some(Item::Image { rect, dest, .. }) = items.get(1) else {
        panic!("{items:?}");
    };
    assert_eq!((dest.w, dest.h), (40.0, 40.0));
    assert_eq!(rect, dest, "contain draws the whole picture");
}

#[test]
fn a_picture_still_decoding_shows_the_placeholder_then_itself() {
    // A big enough PNG that the worker is still busy when the frame is built, most of the time;
    // either way the frame is a placeholder or the picture, never nothing.
    let b = solid_png(1500, 1500, [0, 0, 255, 255]);
    let e = Entry::from_encoded(b, 0).unwrap();
    let mut win = Win::new(|w, c| image_node(w, c, e.id, 2, &[]));
    let pal = Palette::for_env(&win.env);
    let list = win.list();
    let first = list.chunk_items(&list.chunks[0]).to_vec();
    match first.as_slice() {
        [
            Item::Rect {
                fill: Fill::Solid(c),
                ..
            },
        ] => assert_eq!(*c, pal.token(Token::Control)),
        [Item::Image { .. }] => {}
        other => panic!("{other:?}"),
    }
    assert!(settle(WAIT));
    let list = win.list();
    assert!(
        matches!(list.chunk_items(&list.chunks[0]), [Item::Image { .. }]),
        "{:?}",
        list.items
    );
}

#[test]
fn a_headless_window_draws_a_picture_named_by_path() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wolkit-image");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("green.png");
    std::fs::write(&path, solid_png(32, 32, [0, 200, 0, 255])).unwrap();
    let eng =
        wolkit::engine::Engine::new("org.example.pictures", wolkit::abi::ENGINE_HEADLESS).unwrap();
    let win = eng.create_window("Pictures", 64, 64, 0).unwrap();
    let mut w = Writer::new();
    w.create(1, Kind::Image).insert(0, 1, APPEND);
    w.set_prop(1, id::WIDTH, &px(64.0));
    w.set_prop(1, id::HEIGHT, &px(64.0));
    w.set_prop(1, id::IMAGE_FIT, &[2]);
    let s = w.string(path.to_str().unwrap());
    w.set_prop(1, id::IMAGE_SOURCE, &[s]);
    eng.apply(win, w.finish(1)).unwrap();
    let mut out = vec![0u8; 64 * 64 * 4];
    eng.render_rgba(win, &mut out).unwrap();
    let at = |x: usize, y: usize| -> [u8; 4] { out[(y * 64 + x) * 4..][..4].try_into().unwrap() };
    assert_eq!(
        at(32, 32),
        [0, 200, 0, 255],
        "the first frame shows the picture"
    );
    assert_eq!(at(1, 62), [0, 200, 0, 255]);
}
