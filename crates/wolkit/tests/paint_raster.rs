// SPDX-License-Identifier: BSD-2-Clause
//! Properties of the CPU rasteriser on random and hostile display lists: it never panics, the
//! output does not depend on the tile size, and basic compositing laws hold.

use wolkit::cpu::raster::{RasterOptions, render};
use wolkit::paint::geom::{Color, Rect};
use wolkit::paint::glyphs::NoGlyphs;
use wolkit::paint::list::{DisplayList, Fill, Radii, Shadow};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn f(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.next() % 10_000) as f32 / 10_000.0 * (hi - lo)
    }

    fn color(&mut self) -> Color {
        let v = self.next();
        Color::rgba(v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8)
    }

    fn rect(&mut self) -> Rect {
        Rect::new(
            self.f(-30.0, 120.0),
            self.f(-30.0, 120.0),
            self.f(0.0, 90.0),
            self.f(0.0, 90.0),
        )
    }
}

fn random_list(rng: &mut Rng) -> DisplayList {
    let mut l = DisplayList::new();
    let mut clips = 0;
    let mut groups = 0;
    for _ in 0..(5 + rng.next() % 25) {
        match rng.next() % 9 {
            0 | 1 => l.rect(
                rng.rect(),
                Radii::uniform(rng.f(0.0, 40.0)),
                Fill::Solid(rng.color()),
            ),
            2 => {
                let (a, b) = (rng.color(), rng.color());
                l.rect(
                    rng.rect(),
                    Radii([rng.f(0.0, 30.0), 0.0, rng.f(0.0, 30.0), 3.0]),
                    Fill::Linear {
                        from: a,
                        to: b,
                        p0: (rng.f(0.0, 1.0), 0.0),
                        p1: (rng.f(0.0, 1.0), 1.0),
                    },
                );
            }
            3 => {
                let (w, c) = (rng.f(0.0, 12.0), rng.color());
                l.border(rng.rect(), Radii::uniform(rng.f(0.0, 30.0)), w, c);
            }
            4 => {
                let (blur, spread, dx, dy, c) = (
                    rng.f(0.0, 30.0),
                    rng.f(-5.0, 10.0),
                    rng.f(-9.0, 9.0),
                    rng.f(-9.0, 9.0),
                    rng.color(),
                );
                l.shadow(
                    rng.rect(),
                    Radii::uniform(rng.f(0.0, 20.0)),
                    Shadow::new(blur, spread, dx, dy, c),
                );
            }
            5 => {
                l.push_clip(rng.rect(), rng.f(0.0, 25.0));
                clips += 1;
            }
            6 if clips > 0 => {
                l.pop_clip();
                clips -= 1;
            }
            7 => {
                l.push_opacity((rng.next() % 256) as u8, rng.next().is_multiple_of(2));
                groups += 1;
            }
            8 if groups > 0 => {
                l.pop_opacity();
                groups -= 1;
            }
            _ => {}
        }
    }
    l.finish();
    l
}

#[test]
fn random_lists_render_identically_for_any_tile_size() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for case in 0..60 {
        let l = random_list(&mut rng);
        let scale = if case % 3 == 0 { 1.5 } else { 1.0 };
        let o = |tile| RasterOptions {
            scale,
            clear: Color::rgb(240, 240, 245),
            tile,
        };
        let a = render(&l, &mut NoGlyphs, 100, 90, &o(64));
        let b = render(&l, &mut NoGlyphs, 100, 90, &o(13));
        assert_eq!(a, b, "case {case}: tile size changed the output");
        let c = render(&l, &mut NoGlyphs, 100, 90, &o(64));
        assert_eq!(a, c, "case {case}: not deterministic");
    }
}

#[test]
fn hostile_geometry_does_not_panic() {
    let mut l = DisplayList::new();
    let big = 1.0e30_f32;
    for r in [
        Rect::new(-big, -big, 2.0 * big, 2.0 * big),
        Rect::new(f32::NAN, 0.0, 10.0, 10.0),
        Rect::new(0.0, 0.0, f32::INFINITY, 5.0),
        Rect::new(0.0, 0.0, 0.0, 0.0),
        Rect::new(1.0e9, 1.0e9, 5.0, 5.0),
    ] {
        l.rect(
            r,
            Radii([f32::NAN, f32::INFINITY, -1.0, 1e30]),
            Fill::Solid(Color::rgba(9, 9, 9, 200)),
        );
        l.border(r, Radii::uniform(5.0), f32::NAN, Color::rgb(1, 1, 1));
        l.border(r, Radii::uniform(5.0), 1.0e12, Color::rgb(1, 1, 1));
        l.shadow(
            r,
            Radii::ZERO,
            Shadow::new(1.0e12, f32::NAN, f32::INFINITY, 0.0, Color::rgb(0, 0, 0)),
        );
        l.push_clip(r, f32::NAN);
        l.solid(Rect::new(0.0, 0.0, 50.0, 50.0), Color::rgb(5, 5, 5));
    }
    // Far more pops than pushes, and pushes that are never closed.
    for _ in 0..50 {
        l.pop_clip();
        l.pop_opacity();
    }
    for _ in 0..2000 {
        l.push_clip(Rect::new(0.0, 0.0, 40.0, 40.0), 3.0);
    }
    l.solid(Rect::new(0.0, 0.0, 40.0, 40.0), Color::rgb(7, 7, 7));
    l.finish();
    let img = render(&l, &mut NoGlyphs, 64, 64, &RasterOptions::default());
    assert_eq!(img.data.len(), 64 * 64 * 4);
    // Degenerate output sizes.
    assert!(
        render(&l, &mut NoGlyphs, 0, 10, &RasterOptions::default())
            .data
            .is_empty()
    );
    assert!(
        render(&l, &mut NoGlyphs, 10, 0, &RasterOptions::default())
            .data
            .is_empty()
    );
    let nan_scale = RasterOptions {
        scale: f32::NAN,
        ..RasterOptions::default()
    };
    assert_eq!(
        render(&l, &mut NoGlyphs, 8, 8, &nan_scale).data.len(),
        8 * 8 * 4
    );
}

#[test]
fn compositing_laws() {
    let bg = Color::rgb(200, 100, 50);
    let draw = |l: &DisplayList| {
        render(
            l,
            &mut NoGlyphs,
            4,
            4,
            &RasterOptions {
                clear: bg,
                ..RasterOptions::default()
            },
        )
    };
    let whole = Rect::new(0.0, 0.0, 4.0, 4.0);
    // Fully transparent paint changes nothing; fully opaque paint replaces.
    let mut l = DisplayList::new();
    l.solid(whole, Color::rgba(255, 0, 0, 0));
    l.finish();
    assert_eq!(draw(&l).pixel(1, 1), [200, 100, 50, 255]);
    let mut l = DisplayList::new();
    l.solid(whole, Color::rgb(1, 2, 3));
    l.finish();
    assert_eq!(draw(&l).pixel(1, 1), [1, 2, 3, 255]);
    // Black at 50% over a colour halves it (to rounding).
    let mut l = DisplayList::new();
    l.solid(whole, Color::rgba(0, 0, 0, 128));
    l.finish();
    let p = draw(&l).pixel(2, 2);
    assert!(
        (i32::from(p[0]) - 100).abs() <= 1 && (i32::from(p[1]) - 50).abs() <= 1,
        "{p:?}"
    );
    assert_eq!(p[3], 255, "an opaque backdrop stays opaque");
    // Clip excludes everything outside it.
    let mut l = DisplayList::new();
    l.push_clip(Rect::new(1.0, 1.0, 2.0, 2.0), 0.0);
    l.solid(whole, Color::rgb(0, 255, 0));
    l.pop_clip();
    l.finish();
    let img = draw(&l);
    assert_eq!(img.pixel(1, 1), [0, 255, 0, 255]);
    assert_eq!(img.pixel(0, 0), [200, 100, 50, 255]);
    assert_eq!(img.pixel(3, 3), [200, 100, 50, 255]);
    // A transparent clear colour keeps alpha: premultiplied storage, straight output.
    let mut l = DisplayList::new();
    l.solid(Rect::new(0.0, 0.0, 2.0, 4.0), Color::rgba(255, 0, 0, 128));
    l.finish();
    let img = render(&l, &mut NoGlyphs, 4, 4, &RasterOptions::default());
    assert_eq!(img.pixel(0, 0), [128, 0, 0, 128]);
    assert_eq!(&img.to_straight()[..4], &[255, 0, 0, 128]);
    assert_eq!(img.pixel(3, 0), [0, 0, 0, 0]);
}
