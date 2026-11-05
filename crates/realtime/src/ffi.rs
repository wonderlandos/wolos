// SPDX-License-Identifier: GPL-2.0-or-later
//! C ABI for the wol-hypr compositor plugin. Header: `include/wol_realtime.h` (kept in sync by test).
//! All functions are pure or best effort, never panic across the boundary, never allocate per call
//! (except the first `wol_qos_foreground`, which creates the connection).
//!
//! Out-pointers are `Option<&mut T>`: the same ABI as a C `T *` (Rust guarantees the null-pointer
//! optimisation for `Option<&mut T>`, null is `None`), so these functions need no `unsafe`. The C
//! caller passes null or a pointer to a writable, aligned `T` that no other argument points to and
//! nothing else touches during the call (the plugin passes distinct locals, `plugins/wol-hypr`).
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
/// `out` may be null.
#[unsafe(no_mangle)]
pub extern "C" fn wol_anim_sample(
    kind: u32,
    start_ms: u64,
    now_ms: u64,
    out: Option<&mut WolVisual>,
) -> i32 {
    let a = match kind {
        WOL_ANIM_OPEN => Animation::open(start_ms),
        WOL_ANIM_CLOSE => Animation::close(start_ms),
        _ => return -1,
    };
    let v = a.sample(now_ms);
    if let Some(out) = out {
        *out = WolVisual {
            scale: v.scale,
            opacity: v.opacity,
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
    usize::try_from(index)
        .ok()
        .and_then(|i| NAMES.get(i))
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
        // 6: an Xwayland client that draws its own frame (_GTK_FRAME_EXTENTS) is client-side too.
        2 | 6 => Decoration::Client,
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

/// Write `v` through an out-pointer the caller may have left null.
fn put(out: Option<&mut i32>, v: i32) {
    if let Some(out) = out {
        *out = v;
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
/// `x`, `y` may be null.
#[unsafe(no_mangle)]
pub extern "C" fn wol_titlebar_button_origin(
    button: i32,
    x: Option<&mut i32>,
    y: Option<&mut i32>,
) -> i32 {
    let b = match button {
        0 => Button::Close,
        1 => Button::Minimize,
        2 => Button::Zoom,
        _ => return -1,
    };
    let (bx, by) = titlebar::button_origin(b);
    put(x, bx);
    put(y, by);
    0
}

/// Position for a new floating window of `w`x`h` in area (ax, ay, aw, ah) with title bar `bar`.
/// `has_prev` != 0: cascade from (px, py) (previous window of the same app). `x`, `y` may be null.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)] // a C export: every input is a plain argument
pub extern "C" fn wol_place(
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
    x: Option<&mut i32>,
    y: Option<&mut i32>,
) -> i32 {
    let prev = (has_prev != 0).then_some((px, py));
    // (ax, ay, aw, ah) is the output minus reserved zones (menu bar, dock); keep the margin.
    let area = crate::placement::usable((ax, ay, aw, ah), None);
    let (rx, ry) = crate::placement::place((w, h), area, bar, prev);
    put(x, rx);
    put(y, ry);
    0
}

/// Client size for a new floating window: `placement::fit` of its `w`x`h` wish into the zone
/// (ax, ay, aw, ah) minus the placement margin, title bar `bar` included. 0 in min/max = unset.
/// `out_w`, `out_h` may be null.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)] // a C export: every input is a plain argument
pub extern "C" fn wol_fit(
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
    out_w: Option<&mut i32>,
    out_h: Option<&mut i32>,
) -> i32 {
    let area = crate::placement::usable((ax, ay, aw, ah), None);
    let (rw, rh) = crate::placement::fit((w, h), (min_w, min_h), (max_w, max_h), area, bar);
    put(out_w, rw);
    put(out_h, rh);
    0
}

/// Position for a dialog of `w`x`h` centred on its parent's client rect (px, py, pw, ph), kept in
/// the zone (ax, ay, aw, ah) minus the placement margin (`placement::centre_on`). `x`, `y` may be
/// null.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)] // a C export: every input is a plain argument
pub extern "C" fn wol_centre(
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
    x: Option<&mut i32>,
    y: Option<&mut i32>,
) -> i32 {
    let area = crate::placement::usable((ax, ay, aw, ah), None);
    let (rx, ry) = crate::placement::centre_on((w, h), (px, py, pw, ph), area, bar);
    put(x, rx);
    put(y, ry);
    0
}

/// An open floating window after the zone changed (`placement::refit`): client box in/out through
/// `x`, `y`, `w`, `h`. Returns 1 when the box changed, 0 when it already fit or a pointer is null.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)] // a C export: every input is a plain argument
pub extern "C" fn wol_refit(
    min_w: i32,
    min_h: i32,
    max_w: i32,
    max_h: i32,
    ax: i32,
    ay: i32,
    aw: i32,
    ah: i32,
    bar: i32,
    x: Option<&mut i32>,
    y: Option<&mut i32>,
    w: Option<&mut i32>,
    h: Option<&mut i32>,
) -> i32 {
    let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) else {
        return 0;
    };
    let area = crate::placement::usable((ax, ay, aw, ah), None);
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
#[unsafe(no_mangle)]
pub extern "C" fn wol_placement_limits(
    cur_w: i32,
    cur_h: i32,
    min_w: Option<&mut i32>,
    min_h: Option<&mut i32>,
    max_w: Option<&mut i32>,
    max_h: Option<&mut i32>,
) -> i32 {
    let (Some(min_w), Some(min_h), Some(max_w), Some(max_h)) = (min_w, min_h, max_w, max_h) else {
        return -1;
    };
    let (mut min, mut max) = ((*min_w, *min_h), (*max_w, *max_h));
    let f = crate::wm_gates::FixedAxes::of(min, max);
    f.pin((cur_w, cur_h), &mut min, &mut max);
    (*min_w, *min_h, *max_w, *max_h) = (min.0, min.1, max.0, max.1);
    f.bits() as i32
}

/// One watched key (`wm_gates::SwitchKeys`): `state` from the previous call (start at 0), `key`
/// WOL_KEY_*, `flags` WOL_KEYF_*. Returns the next state; `action` (may be null) gets the line
/// code (WOL_LINE_*, text from `wol_keys_line`) in the low byte and WOL_ACTION_CONSUME.
#[unsafe(no_mangle)]
pub extern "C" fn wol_keys_step(state: u32, key: i32, flags: u32, action: Option<&mut u32>) -> u32 {
    use crate::wm_gates::{Key, KeyEvent, SwitchKeys};
    let mut s = SwitchKeys::from_bits(state);
    let a = s.on(Key::from_code(key), KeyEvent::from_flags(flags));
    if let Some(action) = action {
        *action = a.bits();
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

// ---------------------------------------------------------------- window snapping (snap.rs)

/// `snap::zone_at`: the zone under the pointer (px, py) on an output whose box is (sx, sy, sw,
/// sh), `open` its outer edges (WOL_SNAP_OPEN_*). A `snap::Zone` code (0 none).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)] // a C export: every input is a plain argument
pub extern "C" fn wol_snap_zone_at(
    sx: i32,
    sy: i32,
    sw: i32,
    sh: i32,
    open: u32,
    px: i32,
    py: i32,
) -> i32 {
    let screen = crate::Rect {
        x: sx,
        y: sy,
        w: sw,
        h: sh,
    };
    crate::snap::zone_at(screen, open, px, py) as i32
}

/// `snap::zone_box`: the frame of `zone` in the usable area (ax, ay, aw, ah) with the tiling
/// gaps. 0 with x, y, w, h set (each may be null), -1 for zone none or a bad code.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)] // a C export: every input is a plain argument
pub extern "C" fn wol_snap_box(
    zone: i32,
    ax: i32,
    ay: i32,
    aw: i32,
    ah: i32,
    outer: i32,
    inner: i32,
    x: Option<&mut i32>,
    y: Option<&mut i32>,
    w: Option<&mut i32>,
    h: Option<&mut i32>,
) -> i32 {
    let area = crate::Rect {
        x: ax,
        y: ay,
        w: aw,
        h: ah,
    };
    let Some(r) = crate::snap::Zone::from_code(zone)
        .and_then(|z| crate::snap::zone_box(z, area, outer, inner))
    else {
        return -1;
    };
    put(x, r.x);
    put(y, r.y);
    put(w, r.w);
    put(h, r.h);
    0
}

pub const WOL_SNAP_STAY: i32 = 0;
pub const WOL_SNAP_TO: i32 = 1;
pub const WOL_SNAP_RESTORE: i32 = 2;
pub const WOL_SNAP_MINIMISE: i32 = 3;
pub const WOL_SNAP_MOVE_TILED: i32 = 4;
pub const WOL_SNAP_ZOOM_TILED: i32 = 5;

/// `snap::key_step`: one Super+arrow (`key` 0 left, 1 right, 2 up, 3 down) on a window in zone
/// `now`. Step kind (WOL_SNAP_*) in the low byte, the zone (TO) or the key (MOVE_TILED) in the
/// next; -1 for a bad zone or key.
#[unsafe(no_mangle)]
pub extern "C" fn wol_snap_key(now: i32, key: i32, tiled: i32, zoomed: i32) -> i32 {
    use crate::snap::{Key, Step, Zone, key_step};
    let (Some(z), Some(k)) = (Zone::from_code(now), Key::from_code(key)) else {
        return -1;
    };
    match key_step(z, k, tiled != 0, zoomed != 0) {
        Step::Stay => WOL_SNAP_STAY,
        Step::To(z) => WOL_SNAP_TO | (z as i32) << 8,
        Step::Restore => WOL_SNAP_RESTORE,
        Step::Minimise => WOL_SNAP_MINIMISE,
        Step::MoveTiled(k) => WOL_SNAP_MOVE_TILED | (k as i32) << 8,
        Step::ZoomTiled => WOL_SNAP_ZOOM_TILED,
    }
}

/// `snap::unsnap_at`: top-left (x, y; may be null) for a snapped frame (cx, cy, cw, ch) that gets
/// its old size w x h back under the pointer (px, py).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)] // a C export: every input is a plain argument
pub extern "C" fn wol_snap_unsnap(
    cx: i32,
    cy: i32,
    cw: i32,
    ch: i32,
    w: i32,
    h: i32,
    px: i32,
    py: i32,
    x: Option<&mut i32>,
    y: Option<&mut i32>,
) {
    let cur = crate::Rect {
        x: cx,
        y: cy,
        w: cw,
        h: ch,
    };
    let (rx, ry) = crate::snap::unsnap_at(cur, (w, h), px, py);
    put(x, rx);
    put(y, ry);
}

/// `snap::still_at`: 1 when the frame (x, y, w, h) is still the snapped one (sx, sy, sw, sh).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)] // a C export: every input is a plain argument
pub extern "C" fn wol_snap_still(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    sx: i32,
    sy: i32,
    sw: i32,
    sh: i32,
) -> i32 {
    let r = |x, y, w, h| crate::Rect { x, y, w, h };
    i32::from(crate::snap::still_at(r(x, y, w, h), r(sx, sy, sw, sh)))
}

/// Word of zone `code` (`hyprctl wolsnap`, the shell's `snap-preview` line): static,
/// NUL-terminated; null for a bad code.
#[unsafe(no_mangle)]
pub extern "C" fn wol_snap_zone_name(code: i32) -> *const std::ffi::c_char {
    crate::snap::Zone::from_code(code).map_or(std::ptr::null(), |z| z.wire().as_ptr())
}

/// Milliseconds the pointer rests on the zoom button before the snap layouts chooser opens.
#[unsafe(no_mangle)]
pub extern "C" fn wol_snap_layouts_dwell_ms() -> u64 {
    crate::snap::LAYOUTS_DWELL_MS
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
    fn snap_abi_matches_snap_rs() {
        use crate::snap::{Zone, zone_box};
        assert_eq!(
            wol_snap_zone_at(0, 0, 1280, 800, 15, 0, 400),
            Zone::Left as i32
        );
        assert_eq!(wol_snap_zone_at(0, 0, 1280, 800, 15 & !1, 0, 400), 0);
        let (mut x, mut y, mut w, mut h) = (0, 0, 0, 0);
        assert_eq!(
            wol_snap_box(
                Zone::Right as i32,
                0,
                28,
                1280,
                708,
                12,
                6,
                Some(&mut x),
                Some(&mut y),
                Some(&mut w),
                Some(&mut h)
            ),
            0
        );
        let area = crate::Rect {
            x: 0,
            y: 28,
            w: 1280,
            h: 708,
        };
        assert_eq!(
            Some(crate::Rect { x, y, w, h }),
            zone_box(Zone::Right, area, 12, 6)
        );
        assert_eq!(
            wol_snap_box(0, 0, 0, 10, 10, 0, 0, None, None, None, None),
            -1
        );
        assert_eq!(
            wol_snap_box(42, 0, 0, 10, 10, 0, 0, None, None, None, None),
            -1
        );
        assert_eq!(
            wol_snap_key(0, 0, 0, 0),
            WOL_SNAP_TO | (Zone::Left as i32) << 8
        );
        assert_eq!(wol_snap_key(1, 1, 0, 0), WOL_SNAP_RESTORE);
        assert_eq!(wol_snap_key(0, 3, 0, 0), WOL_SNAP_MINIMISE);
        assert_eq!(wol_snap_key(0, 0, 1, 0), WOL_SNAP_MOVE_TILED);
        assert_eq!(wol_snap_key(0, 2, 1, 0), WOL_SNAP_ZOOM_TILED);
        assert_eq!(wol_snap_key(1, 0, 0, 0), WOL_SNAP_STAY);
        assert_eq!(wol_snap_key(9, 0, 0, 0), -1);
        assert_eq!(wol_snap_key(0, 4, 0, 0), -1);
        let (mut ux, mut uy) = (0, 0);
        wol_snap_unsnap(
            12,
            40,
            622,
            684,
            800,
            500,
            12,
            50,
            Some(&mut ux),
            Some(&mut uy),
        );
        assert_eq!((ux, uy), (12, 40));
        assert_eq!(wol_snap_still(1, 2, 3, 4, 1, 2, 3, 4), 1);
        assert_eq!(wol_snap_still(1, 2, 3, 40, 1, 2, 3, 4), 0);
        assert!(wol_snap_zone_name(8).is_null());
        for z in Zone::ALL {
            let p = wol_snap_zone_name(z as i32);
            assert_eq!(p, z.wire().as_ptr());
        }
        assert!(wol_snap_layouts_dwell_ms() >= 300);
    }

    #[test]
    fn abi_matches_pure_modules() {
        let mut v = WolVisual::default();
        assert_eq!(wol_anim_sample(WOL_ANIM_OPEN, 0, 0, Some(&mut v)), 0);
        assert!(v.opacity < 1e-3);
        assert_eq!(wol_anim_sample(WOL_ANIM_OPEN, 0, 10_000, Some(&mut v)), 1);
        assert!((v.scale - 1.0).abs() < 1e-3 && (v.opacity - 1.0).abs() < 1e-3);
        assert_eq!(wol_anim_sample(9, 0, 0, None), -1);
        assert_eq!(
            wol_anim_duration_ms(WOL_ANIM_CLOSE),
            Animation::close(0).duration_ms
        );
        let mut p = [9.0f32; 4];
        for (i, c) in crate::anim::Curve::ALL.into_iter().enumerate() {
            let n =
                unsafe { std::ffi::CStr::from_ptr(wol_anim_curve_name(u32::try_from(i).unwrap())) };
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
        assert_eq!(wol_titlebar_button_origin(2, Some(&mut x), Some(&mut y)), 0);
        assert_eq!((x, y), titlebar::button_origin(Button::Zoom));
        assert_eq!(wol_titlebar_button_origin(7, Some(&mut x), None), -1);
        wol_place(
            400,
            300,
            0,
            0,
            1000,
            800,
            28,
            0,
            0,
            0,
            Some(&mut x),
            Some(&mut y),
        );
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
        wol_fit(
            1600,
            1000,
            0,
            0,
            0,
            0,
            0,
            28,
            1280,
            692,
            30,
            Some(&mut w),
            Some(&mut h),
        );
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
        wol_centre(
            400,
            300,
            300,
            200,
            600,
            400,
            0,
            28,
            1280,
            692,
            30,
            Some(&mut x),
            Some(&mut y),
        );
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
            wol_refit(
                0,
                0,
                0,
                0,
                0,
                28,
                1280,
                692,
                30,
                Some(&mut rx),
                Some(&mut ry),
                Some(&mut rw),
                Some(&mut rh),
            ),
            0,
            "fits: unchanged"
        );
        assert_eq!((rx, ry, rw, rh), (100, 100, 400, 300));
        (rx, ry) = (2000, 2000);
        assert_eq!(
            wol_refit(
                0,
                0,
                0,
                0,
                0,
                28,
                1280,
                692,
                30,
                Some(&mut rx),
                Some(&mut ry),
                Some(&mut rw),
                Some(&mut rh),
            ),
            1
        );
        assert_eq!((rx, ry, rw, rh), (1272 - 400, 720 - 8 - 300, 400, 300));
        assert_eq!(
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
                None,
                Some(&mut ry),
                Some(&mut rw),
                Some(&mut rh),
            ),
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
            wol_placement_limits(
                300,
                900,
                Some(&mut a),
                Some(&mut b),
                Some(&mut c2),
                Some(&mut d)
            ),
            1
        );
        assert_eq!((a, b, c2, d), (300, 200, 300, 0));
        assert_eq!(
            wol_placement_limits(1, 1, None, Some(&mut b), Some(&mut c2), Some(&mut d)),
            -1
        );
        let mut act = 0u32;
        let st = wol_keys_step(0, 0, 1, Some(&mut act)); // Super down
        assert_eq!(act, 0);
        let st = wol_keys_step(st, 1, 1 | 2, Some(&mut act)); // Tab with Super
        assert_eq!(act, 2 | 0x100, "show switcher, consumed");
        // The C side gets the static wire text itself (no copy), NUL-terminated.
        let line = crate::wm_gates::Line::from_code((act & 0xff) as i32).wire();
        assert_eq!(wol_keys_line((act & 0xff) as i32), line.as_ptr());
        assert_eq!(line.to_str().unwrap(), "show switcher\n");
        let st = wol_keys_step(st, 0, 0, None); // Super up
        let _ = wol_keys_step(st, 1, 0, Some(&mut act)); // Tab up: still ours
        assert_eq!(act, 0x100);
        // One unsafe call site for the whole table: `class` is a C string or null, as asked.
        let lights = |class: Option<&std::ffi::CStr>, decoration, transient, fixed| {
            let p = class.map_or(std::ptr::null(), std::ffi::CStr::as_ptr);
            // SAFETY: `p` is null or a NUL-terminated string that outlives the call.
            unsafe { wol_window_lights(p, decoration, transient, fixed) }
        };
        let class = Some(c"org.gnome.Nautilus");
        assert_eq!(lights(class, 0, 0, 0), 0);
        assert_eq!(lights(class, 1, 1, 1), 1);
        assert_eq!(lights(class, 1, 0, 0), 3);
        assert_eq!(lights(class, 2, 0, 0), 0);
        assert_eq!(lights(Some(c"steam"), 3, 0, 0), 0);
        assert_eq!(lights(None, 1, 0, 0), 3);
        assert_eq!(lights(class, 9, 0, 0), 3, "unknown: ours");
        // QA-WM-2: an Xwayland GTK app that draws its own frame (it sets `_GTK_FRAME_EXTENTS`, which
        // wol-hypr reports as decoration 6) keeps its header-bar buttons: a strip over it is a second
        // set of buttons, the title twice and a band between the strip and the app's shadow margin.
        // A plain X11 window still gets ours, a menu or tooltip none.
        let editor = Some(c"org.gnome.TextEditor");
        assert_eq!(lights(editor, 6, 0, 0), 0, "own frame: no strip");
        assert_eq!(lights(editor, 3, 0, 0), 3, "plain X11: ours");
        assert_eq!(lights(editor, 4, 0, 0), 0, "menu, tooltip");
    }

    #[test]
    fn frame_ring_round_trips_through_the_abi() {
        wol_frame_record(0, 2_000_000, 100, 100, 1);
        let mut buf = [0u8; 256];
        let json = |p: *mut u8, len: usize| {
            // SAFETY: every call below passes null or `buf` with `len` <= its 256 bytes.
            unsafe { wol_frame_stats_json(p, len) }
        };
        let n = json(buf.as_mut_ptr(), buf.len());
        let j = std::str::from_utf8(&buf[..n as usize]).unwrap();
        assert!(
            j.contains(r#""full_frames":"#) && j.contains(r#""p50_us":"#),
            "{j}"
        );
        assert_eq!(json(buf.as_mut_ptr(), 4), -1);
        assert_eq!(json(std::ptr::null_mut(), 64), -1);
    }
}
