//! C ABI for the wol-hypr Hyprland plugin. Header: `include/wol_realtime.h` (kept in sync by test).
//! All functions are pure or best effort, never panic across the boundary, never allocate per call
//! (except the first `wol_qos_foreground`, which creates the connection).
use crate::anim::Animation;
use crate::qos::Qos;
use crate::titlebar::{self, Button, Hit};
use std::sync::Mutex;

/// Window visual state at one instant.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WolVisual {
    pub scale: f32,
    pub opacity: f32,
}

pub const WOL_ANIM_OPEN: u32 = 0;
pub const WOL_ANIM_CLOSE: u32 = 1;

/// Sample the open/close curve. Returns 1 when the animation is done, 0 while running, -1 bad kind.
///
/// # Safety
/// `out` must be null or point to a writable `WolVisual`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_anim_sample(
    kind: u32,
    start_ms: u64,
    now_ms: u64,
    out: *mut WolVisual,
) -> i32 {
    let a = match kind {
        WOL_ANIM_OPEN => Animation::open(start_ms),
        WOL_ANIM_CLOSE => Animation::close(start_ms),
        _ => return -1,
    };
    let v = a.sample(now_ms);
    if !out.is_null() {
        // SAFETY: caller contract above.
        unsafe {
            *out = WolVisual {
                scale: v.scale,
                opacity: v.opacity,
            }
        };
    }
    i32::from(a.done(now_ms))
}

/// Animation length in ms for a kind (0 for an unknown kind).
#[unsafe(no_mangle)]
pub extern "C" fn wol_anim_duration_ms(kind: u32) -> u64 {
    match kind {
        WOL_ANIM_OPEN => Animation::open(0).duration_ms,
        WOL_ANIM_CLOSE => Animation::close(0).duration_ms,
        _ => 0,
    }
}

/// Title bar hit: -1 none, 0 close, 1 minimize, 2 zoom, 3 bar.
#[unsafe(no_mangle)]
pub extern "C" fn wol_titlebar_hit(width: i32, x: i32, y: i32) -> i32 {
    match titlebar::hit_test(width, x, y) {
        None => -1,
        Some(Hit::Button(Button::Close)) => 0,
        Some(Hit::Button(Button::Minimize)) => 1,
        Some(Hit::Button(Button::Zoom)) => 2,
        Some(Hit::Bar) => 3,
    }
}

/// Title bar height in logical px.
#[unsafe(no_mangle)]
pub extern "C" fn wol_titlebar_height() -> i32 {
    titlebar::HEIGHT
}

/// Top-left of button 0/1/2 (close/minimize/zoom). Returns 0 on success, -1 bad button.
///
/// # Safety
/// `x`, `y` must be null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_titlebar_button_origin(button: i32, x: *mut i32, y: *mut i32) -> i32 {
    let b = match button {
        0 => Button::Close,
        1 => Button::Minimize,
        2 => Button::Zoom,
        _ => return -1,
    };
    let (bx, by) = titlebar::button_origin(b);
    // SAFETY: caller contract above.
    unsafe {
        if !x.is_null() {
            *x = bx;
        }
        if !y.is_null() {
            *y = by;
        }
    }
    0
}

/// Position for a new floating window of `w`x`h` in area (ax, ay, aw, ah) with title bar `bar`.
/// `has_prev` != 0: cascade from (px, py) (previous window of the same app).
///
/// # Safety
/// `x`, `y` must be null or writable.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn wol_place(
    w: i32,
    h: i32,
    ax: i32,
    ay: i32,
    aw: i32,
    ah: i32,
    bar: i32,
    has_prev: i32,
    px: i32,
    py: i32,
    x: *mut i32,
    y: *mut i32,
) -> i32 {
    let prev = (has_prev != 0).then_some((px, py));
    // (ax, ay, aw, ah) is the output minus reserved zones (menu bar, dock); keep the margin.
    let area = crate::placement::usable((ax, ay, aw, ah), None);
    let (rx, ry) = crate::placement::place((w, h), area, bar, prev);
    // SAFETY: caller contract above.
    unsafe {
        if !x.is_null() {
            *x = rx;
        }
        if !y.is_null() {
            *y = ry;
        }
    }
    0
}

static QOS: Mutex<Option<Qos>> = Mutex::new(None);

/// Report the frontmost pid to wold (QoS UserInteractive boost). 1 = sent, 0 = dropped/unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn wol_qos_foreground(pid: i32) -> i32 {
    let Ok(mut q) = QOS.lock() else { return 0 };
    i32::from(q.get_or_insert_with(Qos::default).foreground(pid))
}

static FRAMES: Mutex<crate::frame_stats::Ring> = Mutex::new(crate::frame_stats::Ring::new());

/// Record one rendered frame (render thread; no allocation). Times are CLOCK_MONOTONIC ns.
#[unsafe(no_mangle)]
pub extern "C" fn wol_frame_record(
    begin_ns: u64,
    end_ns: u64,
    damage_px: u64,
    screen_px: u64,
    passes: u32,
) {
    if let Ok(mut r) = FRAMES.try_lock() {
        r.push(crate::frame_stats::Frame {
            begin_ns,
            end_ns,
            damage_px,
            screen_px,
            passes,
        });
    }
}

/// Frame stats as JSON into `buf` (NUL-terminated). Returns the length without NUL, or -1 if
/// `buf` is null or too small.
///
/// # Safety
/// `buf` must be null or point to `len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_frame_stats_json(buf: *mut u8, len: usize) -> i32 {
    let Ok(r) = FRAMES.lock() else { return -1 };
    let j = r.stats().to_json();
    if buf.is_null() || j.len() + 1 > len {
        return -1;
    }
    // SAFETY: caller contract above; j.len() + 1 <= len.
    unsafe {
        std::ptr::copy_nonoverlapping(j.as_ptr(), buf, j.len());
        *buf.add(j.len()) = 0;
    }
    j.len() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_declares_every_exported_fn() {
        let src = include_str!("ffi.rs");
        let header = include_str!("../include/wol_realtime.h");
        let names: Vec<&str> = src
            .lines()
            .filter_map(|l| {
                l.split_once("extern \"C\" fn ")
                    .map(|(_, r)| r.split('(').next().unwrap())
            })
            .filter(|n| n.starts_with("wol_"))
            .collect();
        assert!(names.len() >= 7, "{names:?}");
        for n in names {
            assert!(
                header.contains(&format!(" {n}(")),
                "{n} missing from wol_realtime.h"
            );
        }
        for c in [
            "WOL_ANIM_OPEN 0",
            "WOL_ANIM_CLOSE 1",
            "float scale;",
            "float opacity;",
        ] {
            assert!(header.contains(c), "{c}");
        }
    }

    #[test]
    fn abi_matches_pure_modules() {
        let mut v = WolVisual::default();
        assert_eq!(unsafe { wol_anim_sample(WOL_ANIM_OPEN, 0, 0, &mut v) }, 0);
        assert!(v.opacity < 1e-3);
        assert_eq!(
            unsafe { wol_anim_sample(WOL_ANIM_OPEN, 0, 10_000, &mut v) },
            1
        );
        assert!((v.scale - 1.0).abs() < 1e-3 && (v.opacity - 1.0).abs() < 1e-3);
        assert_eq!(
            unsafe { wol_anim_sample(9, 0, 0, std::ptr::null_mut()) },
            -1
        );
        assert_eq!(
            wol_anim_duration_ms(WOL_ANIM_CLOSE),
            Animation::close(0).duration_ms
        );
        let (ox, oy) = titlebar::button_origin(Button::Close);
        assert_eq!(wol_titlebar_hit(400, ox + 1, oy + 1), 0);
        assert_eq!(wol_titlebar_hit(400, 200, 5), 3);
        assert_eq!(wol_titlebar_hit(400, -1, 5), -1);
        let (mut x, mut y) = (0, 0);
        assert_eq!(unsafe { wol_titlebar_button_origin(2, &mut x, &mut y) }, 0);
        assert_eq!((x, y), titlebar::button_origin(Button::Zoom));
        assert_eq!(unsafe { wol_titlebar_button_origin(7, &mut x, &mut y) }, -1);
        unsafe { wol_place(400, 300, 0, 0, 1000, 800, 28, 0, 0, 0, &mut x, &mut y) };
        assert_eq!(
            (x, y),
            crate::placement::place(
                (400, 300),
                crate::placement::usable((0, 0, 1000, 800), None),
                28,
                None
            ),
            "the zone gets the placement margin (live: windows touched the menu bar zone)"
        );
        assert_eq!(wol_qos_foreground(0), 0);
    }

    #[test]
    fn frame_ring_round_trips_through_the_abi() {
        wol_frame_record(0, 2_000_000, 100, 100, 1);
        let mut buf = [0u8; 256];
        let n = unsafe { wol_frame_stats_json(buf.as_mut_ptr(), buf.len()) };
        let j = std::str::from_utf8(&buf[..n as usize]).unwrap();
        assert!(
            j.contains(r#""full_frames":"#) && j.contains(r#""p50_us":"#),
            "{j}"
        );
        assert_eq!(unsafe { wol_frame_stats_json(buf.as_mut_ptr(), 4) }, -1);
        assert_eq!(
            unsafe { wol_frame_stats_json(std::ptr::null_mut(), 64) },
            -1
        );
    }
}
