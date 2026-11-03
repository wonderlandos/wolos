// SPDX-License-Identifier: BSD-2-Clause
//! Icons and the natural size of pictures, through a headless engine: an `image` node with
//! `icon:<name>` draws the theme icon in the text colour, a missing icon draws nothing, and an
//! image part with no size of its own takes its picture's size once the header was read.
//! Its own test binary: the icon folder is chosen once per process.

use std::path::PathBuf;

use wolkit::engine::Engine;
use wolkit::scene::Kind;
use wolkit::scene::patch::{APPEND, Writer};
use wolkit::scene::prop::{DIM_PX, id};

const BOX: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="currentColor" d="M4 4H12V12H4Z"/></svg>"#;

fn dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wolkit-icons");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("box-symbolic.svg"), BOX).unwrap();
    d
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(&vec![200u8; (w * h * 4) as usize])
            .unwrap();
    }
    out
}

fn px(v: f32) -> [u32; 2] {
    [DIM_PX, v.to_bits()]
}

fn at(out: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    out[(y * w + x) * 4..][..4].try_into().unwrap()
}

#[test]
fn icons_draw_in_the_text_colour_and_a_missing_one_draws_nothing() {
    wolkit::image::icon::install(vec![dir()]);
    let eng = Engine::new("org.example.icons", wolkit::abi::ENGINE_HEADLESS).unwrap();
    let win = eng.create_window("Icons", 64, 32, 0).unwrap();
    let mut w = Writer::new();
    for (n, name) in [(1u32, "icon:box"), (2, "icon:nothing-here")] {
        w.create(n, Kind::Image).insert(0, n, APPEND);
        w.set_prop(n, id::WIDTH, &px(32.0));
        w.set_prop(n, id::HEIGHT, &px(32.0));
        let s = w.string(name);
        w.set_prop(n, id::IMAGE_SOURCE, &[s]);
    }
    eng.apply(win, w.finish(1)).unwrap();
    let mut out = vec![0u8; 64 * 32 * 4];
    eng.render_rgba(win, &mut out).unwrap();
    let r1 = eng.node_rect(win, 1).unwrap();
    let r2 = eng.node_rect(win, 2).unwrap();
    let mid = |r: [f32; 4]| {
        (
            r[0] as usize + r[2] as usize / 2,
            r[1] as usize + r[3] as usize / 2,
        )
    };
    let corner = |r: [f32; 4]| (r[0] as usize + 1, r[1] as usize + 1);
    let (mx, my) = mid(r1);
    let (cx, cy) = corner(r1);
    assert_ne!(
        at(&out, 64, mx, my),
        at(&out, 64, cx, cy),
        "the box is drawn, the corner is clear"
    );
    let (mx, my) = mid(r2);
    let (cx, cy) = corner(r2);
    assert_eq!(
        at(&out, 64, mx, my),
        at(&out, 64, cx, cy),
        "no such icon: nothing is drawn"
    );
}

#[test]
fn an_image_part_with_no_size_takes_its_pictures_size() {
    let d = dir();
    let path = d.join("wide.png");
    std::fs::write(&path, png(48, 24)).unwrap();
    let eng = Engine::new("org.example.sizes", wolkit::abi::ENGINE_HEADLESS).unwrap();
    let win = eng.create_window("Sizes", 128, 64, 0).unwrap();
    let mut w = Writer::new();
    w.create(1, Kind::Image).insert(0, 1, APPEND);
    // `start`, so the parent does not stretch the part: its own (intrinsic) size shows.
    w.set_prop(1, id::ALIGN_SELF, &[1]);
    let s = w.string(path.to_str().unwrap());
    w.set_prop(1, id::IMAGE_SOURCE, &[s]);
    eng.apply(win, w.finish(1)).unwrap();
    let before = eng.node_rect(win, 1).unwrap();
    assert_eq!(
        (before[2], before[3]),
        (64.0, 64.0),
        "64 px square while the size is unknown"
    );
    let mut out = vec![0u8; 128 * 64 * 4];
    eng.render_rgba(win, &mut out).unwrap();
    let after = eng.node_rect(win, 1).unwrap();
    assert_eq!((after[2], after[3]), (48.0, 24.0), "the picture's own size");
}

#[test]
fn every_symbolic_icon_of_the_design_set_draws() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/icons/symbolic");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return; // the design checkout is not present
    };
    let (mut n, mut bad) = (0, Vec::new());
    for f in rd
        .flatten()
        .filter(|f| f.path().extension().is_some_and(|x| x == "svg"))
    {
        let svg = std::fs::read_to_string(f.path()).unwrap();
        let l = wolkit::image::icon::raster(&svg, 32, wolkit::paint::geom::Color::rgb(0, 0, 0));
        n += 1;
        match l {
            Ok(l) if l.px.chunks(4).any(|p| p[3] > 0) => {}
            _ => bad.push(f.file_name()),
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {n} icons do not draw: {bad:?}",
        bad.len()
    );
}
