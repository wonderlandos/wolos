// SPDX-License-Identifier: GPL-2.0-or-later
//! C ABI for the wol-hypr compositor plugin. Header: `include/wol_realtime.h` (kept in sync by test).
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

/// A named curve of the motion table (`anim::Curve`, the shell's names): 0 and its
/// cubic-bezier control points x1, y1, x2, y2 in `out[0..4]` (`linear` is 0, 0, 1, 1), -1 for a
/// null or unknown name (`out` untouched).
///
/// # Safety
/// `name` must be null or a NUL-terminated string; `out` null or 4 writable floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_anim_curve(name: *const std::ffi::c_char, out: *mut f32) -> i32 {
    if name.is_null() {
        return -1;
    }
    // SAFETY: caller contract above.
    let name = unsafe { std::ffi::CStr::from_ptr(name) };
    let Some(c) = name.to_str().ok().and_then(crate::anim::Curve::from_name) else {
        return -1;
    };
    if !out.is_null() {
        let crate::anim::Bezier(a, b, x, y) = c.bezier();
        // SAFETY: caller contract above.
        unsafe { std::ptr::copy_nonoverlapping([a, b, x, y].as_ptr(), out, 4) };
    }
    0
}

/// Name of curve `index` in table order (static, NUL-terminated), null past the end.
#[unsafe(no_mangle)]
pub extern "C" fn wol_anim_curve_name(index: u32) -> *const std::ffi::c_char {
    const NAMES: [&std::ffi::CStr; 8] = [
        c"ease",
        c"linear",
        c"in",
        c"out",
        c"in-out",
        c"glass",
        c"overshoot",
        c"soft",
    ];
    NAMES
        .get(index as usize)
        .map_or(std::ptr::null(), |n| n.as_ptr())
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

/// Window buttons for a new window (`chrome::lights`): 0 none, 1 close only, 3 all.
/// `decoration`: 0 no xdg-decoration, 1 server side, 2 client side, 3 X11, 4 bare X11 window;
/// anything else counts as server side. `class` may be null.
///
/// # Safety
/// `class` must be null or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_window_lights(
    class: *const std::ffi::c_char,
    decoration: i32,
    transient: i32,
    fixed_size: i32,
) -> i32 {
    use crate::chrome::{self, Decoration};
    let class = if class.is_null() {
        ""
    } else {
        // SAFETY: caller contract above.
        unsafe { std::ffi::CStr::from_ptr(class) }
            .to_str()
            .unwrap_or("")
    };
    let decoration = match decoration {
        0 => Decoration::None,
        2 => Decoration::Client,
        3 => Decoration::X11,
        4 => Decoration::X11Bare,
        5 => Decoration::Kit,
        _ => Decoration::Server,
    };
    chrome::lights(chrome::Facts {
        class,
        decoration,
        transient: transient != 0,
        fixed_size: fixed_size != 0,
    }) as i32
}

/// `titlebar::light_at`: 0 close, 1 minimise, 2 zoom, -1 none.
#[unsafe(no_mangle)]
pub extern "C" fn wol_lights_at(count: i32, x: i32, y: i32) -> i32 {
    use crate::titlebar::{Button, light_at};
    match light_at(count, x, y) {
        Some(Button::Close) => 0,
        Some(Button::Minimize) => 1,
        Some(Button::Zoom) => 2,
        None => -1,
    }
}

/// `titlebar::over_lights`.
#[unsafe(no_mangle)]
pub extern "C" fn wol_lights_over(count: i32, x: i32, y: i32) -> i32 {
    i32::from(crate::titlebar::over_lights(count, x, y))
}

/// Button code of the C ABI: 0 close, 1 minimise, 2 zoom, -1 none.
fn button_code(b: Option<Button>) -> i32 {
    match b {
        Some(Button::Close) => 0,
        Some(Button::Minimize) => 1,
        Some(Button::Zoom) => 2,
        None => -1,
    }
}

/// `titlebar::button_at` on a right-aligned row in a bar `width` px wide (close at the right
/// edge, then zoom, minimise): 0 close, 1 minimise, 2 zoom, -1 none.
#[unsafe(no_mangle)]
pub extern "C" fn wol_lights_at_w(width: i32, count: i32, x: i32, y: i32) -> i32 {
    button_code(titlebar::button_at(
        titlebar::Align::Right,
        width,
        count,
        x,
        y,
    ))
}

/// `titlebar::over_group` on a right-aligned row in a bar `width` px wide.
#[unsafe(no_mangle)]
pub extern "C" fn wol_lights_over_w(width: i32, count: i32, x: i32, y: i32) -> i32 {
    i32::from(titlebar::over_group(
        titlebar::Align::Right,
        width,
        count,
        x,
        y,
    ))
}

/// The button in `slot` of a right-aligned row (slot 0 at the right edge; a window showing `n`
/// buttons shows slots 0..n): 0 close, 1 minimise, 2 zoom, -1 bad slot.
#[unsafe(no_mangle)]
pub extern "C" fn wol_lights_slot_w(slot: i32) -> i32 {
    let order = titlebar::Align::Right.order();
    button_code(
        usize::try_from(slot)
            .ok()
            .and_then(|i| order.get(i).copied()),
    )
}

/// `titlebar::Hover::next` on the bits the bar keeps (start at 0).
#[unsafe(no_mangle)]
pub extern "C" fn wol_lights_hover(state: u32, over: i32, held: i32) -> u32 {
    crate::titlebar::Hover::from_bits(state)
        .next(over != 0, held != 0)
        .bits()
}

/// `titlebar::Hover::glyphs` of those bits: 1 shows the glyphs on every light.
#[unsafe(no_mangle)]
pub extern "C" fn wol_lights_glyphs(state: u32) -> i32 {
    i32::from(crate::titlebar::Hover::from_bits(state).glyphs())
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

/// Client size for a new floating window: `placement::fit` of its `w`x`h` wish into the zone
/// (ax, ay, aw, ah) minus the placement margin, title bar `bar` included. 0 in min/max = unset.
///
/// # Safety
/// `out_w`, `out_h` must be null or writable.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn wol_fit(
    w: i32,
    h: i32,
    min_w: i32,
    min_h: i32,
    max_w: i32,
    max_h: i32,
    ax: i32,
    ay: i32,
    aw: i32,
    ah: i32,
    bar: i32,
    out_w: *mut i32,
    out_h: *mut i32,
) -> i32 {
    let area = crate::placement::usable((ax, ay, aw, ah), None);
    let (rw, rh) = crate::placement::fit((w, h), (min_w, min_h), (max_w, max_h), area, bar);
    // SAFETY: caller contract above.
    unsafe {
        if !out_w.is_null() {
            *out_w = rw;
        }
        if !out_h.is_null() {
            *out_h = rh;
        }
    }
    0
}

/// Position for a dialog of `w`x`h` centred on its parent's client rect (px, py, pw, ph), kept in
/// the zone (ax, ay, aw, ah) minus the placement margin (`placement::centre_on`).
///
/// # Safety
/// `x`, `y` must be null or writable.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn wol_centre(
    w: i32,
    h: i32,
    px: i32,
    py: i32,
    pw: i32,
    ph: i32,
    ax: i32,
    ay: i32,
    aw: i32,
    ah: i32,
    bar: i32,
    x: *mut i32,
    y: *mut i32,
) -> i32 {
    let area = crate::placement::usable((ax, ay, aw, ah), None);
    let (rx, ry) = crate::placement::centre_on((w, h), (px, py, pw, ph), area, bar);
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

/// An open floating window after the zone changed (`placement::refit`): client box in/out through
/// `x`, `y`, `w`, `h`. Returns 1 when the box changed, 0 when it already fit.
///
/// # Safety
/// `x`, `y`, `w`, `h` must point to writable i32s.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn wol_refit(
    min_w: i32,
    min_h: i32,
    max_w: i32,
    max_h: i32,
    ax: i32,
    ay: i32,
    aw: i32,
    ah: i32,
    bar: i32,
    x: *mut i32,
    y: *mut i32,
    w: *mut i32,
    h: *mut i32,
) -> i32 {
    if x.is_null() || y.is_null() || w.is_null() || h.is_null() {
        return 0;
    }
    let area = crate::placement::usable((ax, ay, aw, ah), None);
    // SAFETY: caller contract above.
    unsafe {
        let before = (*x, *y, *w, *h);
        let (rx, ry, rw, rh) = crate::placement::refit(
            (before.0, before.1),
            (before.2, before.3),
            (min_w, min_h),
            (max_w, max_h),
            area,
            bar,
        );
        (*x, *y, *w, *h) = (rx, ry, rw, rh);
        i32::from(before != (rx, ry, rw, rh))
    }
}

/// New floating window (`wm_gates::placement_for`): 0 skip (X11 override-redirect or borderless
/// popup), 1 only clamp (an X11 window that asked for its position), 2 place.
#[unsafe(no_mangle)]
pub extern "C" fn wol_x11_placement(
    x11: i32,
    override_redirect: i32,
    no_borders: i32,
    size_hint_flags: u32,
) -> i32 {
    crate::wm_gates::placement_for(
        x11 != 0,
        override_redirect != 0,
        no_borders != 0,
        size_hint_flags,
    ) as i32
}

/// 1 when the plugin may move the window later (cascade origin, refit): `wm_gates::managed`.
#[unsafe(no_mangle)]
pub extern "C" fn wol_x11_managed(x11: i32, override_redirect: i32, no_borders: i32) -> i32 {
    i32::from(crate::wm_gates::managed(
        x11 != 0,
        override_redirect != 0,
        no_borders != 0,
    ))
}

/// Fixed axes from client limits (0 = unset): bit 1 width, bit 2 height.
#[unsafe(no_mangle)]
pub extern "C" fn wol_fixed_axes(min_w: i32, min_h: i32, max_w: i32, max_h: i32) -> u32 {
    crate::wm_gates::FixedAxes::of((min_w, min_h), (max_w, max_h)).bits()
}

/// Limits for `wol_fit`/`wol_refit` in/out: a fixed axis is pinned to the current size
/// (`cur_w`, `cur_h`), the other stays free. Returns the fixed-axes bits, -1 on a null pointer.
///
/// # Safety
/// `min_w`, `min_h`, `max_w`, `max_h` must point to writable i32s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_placement_limits(
    cur_w: i32,
    cur_h: i32,
    min_w: *mut i32,
    min_h: *mut i32,
    max_w: *mut i32,
    max_h: *mut i32,
) -> i32 {
    if min_w.is_null() || min_h.is_null() || max_w.is_null() || max_h.is_null() {
        return -1;
    }
    // SAFETY: caller contract above.
    unsafe {
        let (mut min, mut max) = ((*min_w, *min_h), (*max_w, *max_h));
        let f = crate::wm_gates::FixedAxes::of(min, max);
        f.pin((cur_w, cur_h), &mut min, &mut max);
        (*min_w, *min_h, *max_w, *max_h) = (min.0, min.1, max.0, max.1);
        f.bits() as i32
    }
}

/// One watched key (`wm_gates::SwitchKeys`): `state` from the previous call (start at 0), `key`
/// WOL_KEY_*, `flags` WOL_KEYF_*. Returns the next state; `action` (may be null) gets the line
/// code (WOL_LINE_*, text from `wol_keys_line`) in the low byte and WOL_ACTION_CONSUME.
///
/// # Safety
/// `action` is null or points to a writable u32.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_keys_step(state: u32, key: i32, flags: u32, action: *mut u32) -> u32 {
    use crate::wm_gates::{Key, KeyEvent, SwitchKeys};
    let mut s = SwitchKeys::from_bits(state);
    let a = s.on(Key::from_code(key), KeyEvent::from_flags(flags));
    if !action.is_null() {
        // SAFETY: caller contract above.
        unsafe { *action = a.bits() };
    }
    s.bits()
}

/// Wire text of a WOL_LINE_* code for `wol-shell.sock`, NUL-terminated, static ("" for none).
#[unsafe(no_mangle)]
pub extern "C" fn wol_keys_line(code: i32) -> *const std::ffi::c_char {
    crate::wm_gates::Line::from_code(code).wire().as_ptr()
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
                header.contains(&format!(" {n}(")) || header.contains(&format!("*{n}(")),
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
        let mut p = [9.0f32; 4];
        for (i, c) in crate::anim::Curve::ALL.into_iter().enumerate() {
            let n = unsafe { std::ffi::CStr::from_ptr(wol_anim_curve_name(i as u32)) };
            assert_eq!(n.to_str(), Ok(c.name()));
            assert_eq!(unsafe { wol_anim_curve(n.as_ptr(), p.as_mut_ptr()) }, 0);
            let crate::anim::Bezier(a, b, x, y) = c.bezier();
            assert_eq!(p, [a, b, x, y]);
        }
        assert!(wol_anim_curve_name(8).is_null());
        assert_eq!(
            unsafe { wol_anim_curve(c"wolOut".as_ptr(), p.as_mut_ptr()) },
            -1
        );
        assert_eq!(
            unsafe { wol_anim_curve(std::ptr::null(), p.as_mut_ptr()) },
            -1
        );
        assert_eq!(
            unsafe { wol_anim_curve(c"soft".as_ptr(), std::ptr::null_mut()) },
            0
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
        let (mut w, mut h) = (0, 0);
        unsafe { wol_fit(1600, 1000, 0, 0, 0, 0, 0, 28, 1280, 692, 30, &mut w, &mut h) };
        assert_eq!(
            (w, h),
            crate::placement::fit(
                (1600, 1000),
                (0, 0),
                (0, 0),
                crate::placement::usable((0, 28, 1280, 692), None),
                30
            )
        );
        unsafe {
            wol_centre(
                400, 300, 300, 200, 600, 400, 0, 28, 1280, 692, 30, &mut x, &mut y,
            )
        };
        assert_eq!(
            (x, y),
            crate::placement::centre_on(
                (400, 300),
                (300, 200, 600, 400),
                crate::placement::usable((0, 28, 1280, 692), None),
                30
            )
        );
        let (mut rx, mut ry, mut rw, mut rh) = (100, 100, 400, 300);
        assert_eq!(
            unsafe {
                wol_refit(
                    0, 0, 0, 0, 0, 28, 1280, 692, 30, &mut rx, &mut ry, &mut rw, &mut rh,
                )
            },
            0,
            "fits: unchanged"
        );
        assert_eq!((rx, ry, rw, rh), (100, 100, 400, 300));
        (rx, ry) = (2000, 2000);
        assert_eq!(
            unsafe {
                wol_refit(
                    0, 0, 0, 0, 0, 28, 1280, 692, 30, &mut rx, &mut ry, &mut rw, &mut rh,
                )
            },
            1
        );
        assert_eq!((rx, ry, rw, rh), (1272 - 400, 720 - 8 - 300, 400, 300));
        assert_eq!(
            unsafe {
                wol_refit(
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    10,
                    10,
                    0,
                    std::ptr::null_mut(),
                    &mut ry,
                    &mut rw,
                    &mut rh,
                )
            },
            0,
            "null box: nothing to do"
        );
        assert_eq!(wol_qos_foreground(0), 0);
        assert_eq!(wol_lights_at(3, 67, 14), 2);
        assert_eq!(wol_lights_at(1, 67, 14), -1);
        assert_eq!(wol_lights_over(3, 34, 14), 1);
        assert_eq!(wol_lights_over(3, 300, 14), 0);
        let on = wol_lights_hover(0, 1, 0);
        assert_eq!(wol_lights_glyphs(on), 1);
        assert_eq!(wol_lights_glyphs(wol_lights_hover(on, 0, 0)), 0);
        assert_eq!(wol_lights_glyphs(wol_lights_hover(on, 0, 1)), 1);
        // Right-aligned row: placement and hit follow the bar width; the old calls are unchanged.
        assert_eq!(wol_lights_at_w(600, 3, 577, 14), 0);
        assert_eq!(wol_lights_at_w(600, 3, 555, 14), 2);
        assert_eq!(wol_lights_at_w(600, 3, 533, 14), 1);
        assert_eq!(wol_lights_at_w(600, 1, 555, 14), -1);
        assert_eq!(wol_lights_at_w(600, 3, 23, 14), -1);
        assert_eq!(wol_lights_at_w(800, 3, 777, 14), 0);
        assert_eq!(wol_lights_over_w(600, 3, 566, 14), 1);
        assert_eq!(wol_lights_over_w(600, 3, 300, 14), 0);
        assert_eq!(wol_lights_over_w(600, 0, 577, 14), 0);
        assert_eq!(
            [0, 1, 2, 3, -1].map(|s| wol_lights_slot_w(s)),
            [0, 2, 1, -1, -1],
            "close, zoom, minimise from the right edge"
        );
        // wm_gates through the C ABI (audit4 S5-2, S5-3, S5-11)
        assert_eq!(wol_x11_placement(1, 1, 0, 0), 0);
        assert_eq!(wol_x11_placement(1, 0, 0, 1), 1);
        assert_eq!(wol_x11_placement(0, 1, 1, 5), 2);
        assert_eq!((wol_x11_managed(1, 0, 1), wol_x11_managed(1, 0, 0)), (0, 1));
        assert_eq!(wol_fixed_axes(300, 200, 300, 0), 1);
        let (mut a, mut b, mut c2, mut d) = (300, 200, 300, 0);
        assert_eq!(
            unsafe { wol_placement_limits(300, 900, &mut a, &mut b, &mut c2, &mut d) },
            1
        );
        assert_eq!((a, b, c2, d), (300, 200, 300, 0));
        let null = std::ptr::null_mut();
        assert_eq!(
            unsafe { wol_placement_limits(1, 1, null, &mut b, &mut c2, &mut d) },
            -1
        );
        let mut act = 0u32;
        let st = unsafe { wol_keys_step(0, 0, 1, &mut act) }; // Super down
        assert_eq!(act, 0);
        let st = unsafe { wol_keys_step(st, 1, 1 | 2, &mut act) }; // Tab with Super
        assert_eq!(act, 2 | 0x100, "show switcher, consumed");
        let line = unsafe { std::ffi::CStr::from_ptr(wol_keys_line((act & 0xff) as i32)) };
        assert_eq!(line.to_str().unwrap(), "show switcher\n");
        let st = unsafe { wol_keys_step(st, 0, 0, std::ptr::null_mut()) }; // Super up
        let _ = unsafe { wol_keys_step(st, 1, 0, &mut act) }; // Tab up: still ours
        assert_eq!(act, 0x100);
        // One unsafe call site for the whole table: `class` is a C string or null, as asked.
        let lights = |class: Option<&std::ffi::CStr>, decoration| {
            let p = class.map_or(std::ptr::null(), std::ffi::CStr::as_ptr);
            // SAFETY: `p` is null or a NUL-terminated string that outlives the call.
            unsafe { wol_window_lights(p, decoration, 0, 0) }
        };
        let class = Some(c"org.gnome.Nautilus");
        assert_eq!(lights(class, 0), 0);
        // SAFETY: a NUL-terminated string literal.
        assert_eq!(
            unsafe { wol_window_lights(c"org.gnome.Nautilus".as_ptr(), 1, 1, 1) },
            1
        );
        assert_eq!(lights(class, 1), 3);
        assert_eq!(lights(class, 2), 0);
        assert_eq!(lights(Some(c"steam"), 3), 0);
        assert_eq!(lights(None, 1), 3);
        assert_eq!(lights(class, 9), 3, "unknown: ours");
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
