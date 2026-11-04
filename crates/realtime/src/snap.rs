// SPDX-License-Identifier: GPL-2.0-or-later
//! Window snapping (edge and corner snapping for floating windows): drag
//! a window by its bar to the left or right edge of the screen for a half, to a corner for a
//! quarter, to the top for zoom; Super+arrows walk the same places. Pure: the wol-hypr plugin
//! passes the pointer, the output and its usable area and applies the box it gets back
//! (`plugins/wol-hypr`, `wol_snap_*` in `ffi.rs`). Boxes are frames (title bar included).
use crate::Rect;

/// Where a snapped window sits. `Zoom` is the compositor's maximise (the band between the menu bar
/// and the Dock, no gaps), which the title bar's zoom button also gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Zone {
    None = 0,
    Left = 1,
    Right = 2,
    Zoom = 3,
    TopLeft = 4,
    TopRight = 5,
    BottomLeft = 6,
    BottomRight = 7,
}

impl Zone {
    pub const ALL: [Zone; 8] = [
        Zone::None,
        Zone::Left,
        Zone::Right,
        Zone::Zoom,
        Zone::TopLeft,
        Zone::TopRight,
        Zone::BottomLeft,
        Zone::BottomRight,
    ];

    pub fn from_code(c: i32) -> Option<Zone> {
        Zone::ALL.into_iter().find(|z| *z as i32 == c)
    }

    /// Word of the `hyprctl wolsnap` command and of the shell's `snap-preview` line.
    pub fn name(self) -> &'static str {
        self.wire().to_str().unwrap_or_default()
    }

    /// [`Zone::name`] as a static C string (the C ABI hands it out).
    pub fn wire(self) -> &'static std::ffi::CStr {
        match self {
            Zone::None => c"none",
            Zone::Left => c"left",
            Zone::Right => c"right",
            Zone::Zoom => c"zoom",
            Zone::TopLeft => c"top-left",
            Zone::TopRight => c"top-right",
            Zone::BottomLeft => c"bottom-left",
            Zone::BottomRight => c"bottom-right",
        }
    }

    pub fn from_name(s: &str) -> Option<Zone> {
        Zone::ALL.into_iter().find(|z| z.name() == s)
    }
}

/// Pointer distance from a screen edge (px) that counts as touching it. The pointer stops at an
/// outer edge, so a few px forgive a touchpad that does not quite reach it.
pub const EDGE: i32 = 6;
/// Along a side edge, this close to the top or bottom of the screen is the corner (a quarter).
pub const CORNER: i32 = 96;

/// Outer edges of an output: no other output beyond them (the pointer stops there). An edge shared
/// with a second display is a way across, never a snap.
pub const OPEN_LEFT: u32 = 1;
pub const OPEN_RIGHT: u32 = 2;
pub const OPEN_TOP: u32 = 4;
pub const OPEN_BOTTOM: u32 = 8;

/// The zone under the pointer `(px, py)` on an output whose box is `screen`, `open` its outer
/// edges (`OPEN_*`). Corners win over sides: the left edge near the top is the top-left quarter;
/// the top edge near a side is that quarter too, the rest of the top is zoom.
pub fn zone_at(screen: Rect, open: u32, px: i32, py: i32) -> Zone {
    // Saturating throughout: this runs inside the compositor through the C ABI, where a debug
    // overflow panic would abort it; a bad box only ever gives `None`.
    let (right_edge, bottom_edge) = (
        screen.x.saturating_add(screen.w),
        screen.y.saturating_add(screen.h),
    );
    if px < screen.x || py < screen.y || px >= right_edge || py >= bottom_edge {
        return Zone::None;
    }
    let left = open & OPEN_LEFT != 0 && px < screen.x.saturating_add(EDGE);
    let right = open & OPEN_RIGHT != 0 && px >= right_edge.saturating_sub(EDGE);
    let top = open & OPEN_TOP != 0 && py < screen.y.saturating_add(EDGE);
    let near_top = py < screen.y.saturating_add(CORNER);
    let near_bottom = py >= bottom_edge.saturating_sub(CORNER);
    let near_left = px < screen.x.saturating_add(CORNER);
    let near_right = px >= right_edge.saturating_sub(CORNER);
    match (left, right, top) {
        (true, _, _) if near_top => Zone::TopLeft,
        (true, _, _) if near_bottom => Zone::BottomLeft,
        (true, _, _) => Zone::Left,
        (_, true, _) if near_top => Zone::TopRight,
        (_, true, _) if near_bottom => Zone::BottomRight,
        (_, true, _) => Zone::Right,
        (_, _, true) if near_left => Zone::TopLeft,
        (_, _, true) if near_right => Zone::TopRight,
        (_, _, true) => Zone::Zoom,
        _ => Zone::None,
    }
}

/// The frame a window takes in `zone` inside the usable area `area` (below the menu bar, above the
/// Dock): `outer` px from the area's edges and `inner` px on each side of a split, so two halves
/// sit `2 * inner` apart, the gaps tiled windows have (`general:gaps_out`, `gaps_in`). Zoom is the
/// whole area (zoom has no gaps). `None` for `Zone::None`.
pub fn zone_box(zone: Zone, area: Rect, outer: i32, inner: i32) -> Option<Rect> {
    let (outer, inner) = (outer.clamp(0, 64), inner.clamp(0, 64));
    let (x0, y0) = (area.x.saturating_add(outer), area.y.saturating_add(outer));
    let (w, h) = (
        area.w.saturating_sub(2 * outer).max(2),
        area.h.saturating_sub(2 * outer).max(2),
    );
    // The first half gets the floor, the second the rest: the pair fills the width exactly.
    let split = |len: i32| {
        let a = (len.saturating_sub(2 * inner) / 2).max(1);
        (a, len.saturating_sub(2 * inner).saturating_sub(a).max(1))
    };
    let (lw, rw) = split(w);
    let (th, bh) = split(h);
    let (rx, by) = (
        x0.saturating_add(lw).saturating_add(2 * inner),
        y0.saturating_add(th).saturating_add(2 * inner),
    );
    let r = |x, y, w, h| Some(Rect { x, y, w, h });
    match zone {
        Zone::None => None,
        Zone::Zoom => Some(area),
        Zone::Left => r(x0, y0, lw, h),
        Zone::Right => r(rx, y0, rw, h),
        Zone::TopLeft => r(x0, y0, lw, th),
        Zone::TopRight => r(rx, y0, rw, th),
        Zone::BottomLeft => r(x0, by, lw, bh),
        Zone::BottomRight => r(rx, by, rw, bh),
    }
}

/// Super+arrow keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Key {
    Left = 0,
    Right = 1,
    Up = 2,
    Down = 3,
}

impl Key {
    pub fn from_code(c: i32) -> Option<Key> {
        [Key::Left, Key::Right, Key::Up, Key::Down]
            .into_iter()
            .find(|k| *k as i32 == c)
    }

    pub fn from_name(s: &str) -> Option<Key> {
        match s {
            "l" | "left" => Some(Key::Left),
            "r" | "right" => Some(Key::Right),
            "u" | "up" => Some(Key::Up),
            "d" | "down" => Some(Key::Down),
            _ => None,
        }
    }
}

/// What a Super+arrow does to the focused window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Snap (or zoom) the floating window there.
    To(Zone),
    /// Back to the size and place it had before it snapped.
    Restore,
    Minimise,
    /// Nothing changes (already as far as that key goes).
    Stay,
    /// A tiled window moves through the tiling in that direction (the layout's movewindow).
    MoveTiled(Key),
    /// A tiled window: zoom on, or off again.
    ZoomTiled,
}

/// One Super+arrow from `now` (the zone the window is in, `None` when it is not snapped), the
/// usual walk: Left/Right give a half, and from a half the other way restores; Up/Down
/// turn a half into its top/bottom quarter and a quarter back into its half; Up zooms a free
/// window, Down restores a zoomed one, and minimises a free window or a bottom quarter.
/// `tiled`: the window is part of the tiling (Settings > Tile windows), which owns its place.
pub fn key_step(now: Zone, key: Key, tiled: bool, zoomed: bool) -> Step {
    use Zone::*;
    if tiled {
        return match key {
            Key::Up if !zoomed => Step::ZoomTiled,
            Key::Down if zoomed => Step::ZoomTiled,
            Key::Up | Key::Down => Step::Stay,
            k => Step::MoveTiled(k),
        };
    }
    let now = if zoomed { Zoom } else { now };
    let to = |z: Zone| if z == now { Step::Stay } else { Step::To(z) };
    match (key, now) {
        (Key::Left, None | Zoom | Left) => to(Left),
        (Key::Left, Right) => Step::Restore,
        (Key::Left, TopRight | TopLeft) => to(TopLeft),
        (Key::Left, BottomRight | BottomLeft) => to(BottomLeft),
        (Key::Right, None | Zoom | Right) => to(Right),
        (Key::Right, Left) => Step::Restore,
        (Key::Right, TopLeft | TopRight) => to(TopRight),
        (Key::Right, BottomLeft | BottomRight) => to(BottomRight),
        (Key::Up, None | Zoom) => to(Zoom),
        (Key::Up, Left | TopLeft) => to(TopLeft),
        (Key::Up, Right | TopRight) => to(TopRight),
        (Key::Up, BottomLeft) => Step::To(Left),
        (Key::Up, BottomRight) => Step::To(Right),
        (Key::Down, Zoom) => Step::Restore,
        (Key::Down, Left) => Step::To(BottomLeft),
        (Key::Down, Right) => Step::To(BottomRight),
        (Key::Down, TopLeft) => Step::To(Left),
        (Key::Down, TopRight) => Step::To(Right),
        (Key::Down, None | BottomLeft | BottomRight) => Step::Minimise,
    }
}

/// A snapped window that is dragged away gets its old size back under the pointer: the pointer
/// keeps its place across the bar in proportion, and its distance from the frame top. `cur` is the
/// snapped frame, `size` the frame size to restore; returns the new frame's top-left.
pub fn unsnap_at(cur: Rect, size: (i32, i32), px: i32, py: i32) -> (i32, i32) {
    let rel = if cur.w > 0 {
        f64::from(px.saturating_sub(cur.x).clamp(0, cur.w)) / f64::from(cur.w)
    } else {
        0.5
    };
    let x = px.saturating_sub((rel * f64::from(size.0.max(1))).round() as i32);
    let dy = py.saturating_sub(cur.y).clamp(0, size.1.max(1) - 1);
    (x, py.saturating_sub(dy))
}

/// The window is still where it snapped (nobody moved or resized it since): only then does a
/// drag or Super+arrow treat it as snapped. A tolerance of 2 px absorbs rounding at fractional
/// scales.
pub fn still_at(cur: Rect, snapped: Rect) -> bool {
    let near = |a: i32, b: i32| a.abs_diff(b) <= 2;
    near(cur.x, snapped.x)
        && near(cur.y, snapped.y)
        && near(cur.w, snapped.w)
        && near(cur.h, snapped.h)
}

/// The six places of the snap layouts chooser (shown over the zoom button), in reading order:
/// halves, then the 2x2 quarters.
pub const LAYOUTS: [Zone; 6] = [
    Zone::Left,
    Zone::Right,
    Zone::TopLeft,
    Zone::TopRight,
    Zone::BottomLeft,
    Zone::BottomRight,
];

/// How long the pointer rests on the zoom button before the chooser opens (Windows: about 0.5 s).
pub const LAYOUTS_DWELL_MS: u64 = 600;

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        w: 1280,
        h: 800,
    };
    /// 28 px menu bar, 64 px Dock.
    const AREA: Rect = Rect {
        x: 0,
        y: 28,
        w: 1280,
        h: 708,
    };
    const ALL_OPEN: u32 = OPEN_LEFT | OPEN_RIGHT | OPEN_TOP | OPEN_BOTTOM;

    #[test]
    fn edges_give_halves_corners_quarters_top_zoom() {
        for (px, py, want) in [
            (0, 400, Zone::Left),
            (5, 400, Zone::Left),
            (6, 400, Zone::None),
            (1279, 400, Zone::Right),
            (1274, 400, Zone::Right),
            (640, 0, Zone::Zoom),
            (640, 5, Zone::Zoom),
            (640, 6, Zone::None),
            (0, 10, Zone::TopLeft),
            (0, 95, Zone::TopLeft),
            (0, 96, Zone::Left),
            (1279, 20, Zone::TopRight),
            (0, 799, Zone::BottomLeft),
            (0, 704, Zone::BottomLeft),
            (0, 703, Zone::Left),
            (1279, 760, Zone::BottomRight),
            (20, 0, Zone::TopLeft),
            (1270, 0, Zone::TopRight),
            (640, 799, Zone::None), // the bottom edge alone is the Dock's
            (640, 400, Zone::None),
            (-1, 400, Zone::None),
            (1280, 400, Zone::None),
        ] {
            assert_eq!(zone_at(SCREEN, ALL_OPEN, px, py), want, "({px}, {py})");
        }
    }

    #[test]
    fn an_edge_shared_with_another_display_never_snaps() {
        let open = ALL_OPEN & !OPEN_RIGHT;
        assert_eq!(zone_at(SCREEN, open, 1279, 400), Zone::None);
        assert_eq!(zone_at(SCREEN, open, 1279, 10), Zone::None);
        assert_eq!(zone_at(SCREEN, open, 1270, 0), Zone::TopRight);
        assert_eq!(zone_at(SCREEN, ALL_OPEN & !OPEN_TOP, 640, 0), Zone::None);
        // A second output to the right: its own left edge is shared too.
        let second = Rect { x: 1280, ..SCREEN };
        assert_eq!(
            zone_at(second, ALL_OPEN & !OPEN_LEFT, 1280, 400),
            Zone::None
        );
        assert_eq!(zone_at(second, ALL_OPEN, 2559, 400), Zone::Right);
    }

    #[test]
    fn halves_and_quarters_fill_the_area_with_the_tiling_gaps() {
        let b = |z| zone_box(z, AREA, 12, 6).unwrap();
        let (l, r) = (b(Zone::Left), b(Zone::Right));
        assert_eq!(
            l,
            Rect {
                x: 12,
                y: 40,
                w: 622,
                h: 684
            }
        );
        assert_eq!(r.x - (l.x + l.w), 12, "two halves are 2 * gaps_in apart");
        assert_eq!(
            r.x + r.w,
            AREA.w - 12,
            "the right half ends at the outer gap"
        );
        let (tl, br) = (b(Zone::TopLeft), b(Zone::BottomRight));
        assert_eq!((tl.x, tl.y, tl.w), (12, 40, 622));
        assert_eq!(br.y - (tl.y + tl.h), 12);
        assert_eq!(br.y + br.h, AREA.y + AREA.h - 12);
        assert_eq!((b(Zone::TopRight).x, b(Zone::BottomLeft).y), (r.x, br.y));
        assert_eq!(b(Zone::Zoom), AREA, "zoom is the band, no gaps");
        assert_eq!(zone_box(Zone::None, AREA, 12, 6), None);
        // Odd widths: the halves still meet the outer gaps exactly.
        let odd = Rect { w: 1281, ..AREA };
        let (l, r) = (
            zone_box(Zone::Left, odd, 12, 6).unwrap(),
            zone_box(Zone::Right, odd, 12, 6).unwrap(),
        );
        assert_eq!(r.x + r.w, 1281 - 12);
        assert_eq!(r.x - (l.x + l.w), 12);
        // No gaps (Settings gaps at Narrow): halves touch.
        let (l, r) = (
            zone_box(Zone::Left, AREA, 0, 0).unwrap(),
            zone_box(Zone::Right, AREA, 0, 0).unwrap(),
        );
        assert_eq!((l.x, l.w, r.x, r.w), (0, 640, 640, 640));
    }

    #[test]
    fn super_arrows_walk_halves_quarters_zoom_like_windows() {
        use Step::*;
        use Zone as Z;
        let k = |z, key| key_step(z, key, false, false);
        assert_eq!(k(Z::None, Key::Left), To(Z::Left));
        assert_eq!(k(Z::None, Key::Right), To(Z::Right));
        assert_eq!(k(Z::Left, Key::Right), Restore);
        assert_eq!(k(Z::Right, Key::Left), Restore);
        assert_eq!(k(Z::Left, Key::Left), Stay);
        assert_eq!(k(Z::Left, Key::Up), To(Z::TopLeft));
        assert_eq!(k(Z::Left, Key::Down), To(Z::BottomLeft));
        assert_eq!(k(Z::TopLeft, Key::Down), To(Z::Left));
        assert_eq!(k(Z::BottomLeft, Key::Up), To(Z::Left));
        assert_eq!(k(Z::TopLeft, Key::Right), To(Z::TopRight));
        assert_eq!(k(Z::BottomRight, Key::Left), To(Z::BottomLeft));
        assert_eq!(k(Z::BottomRight, Key::Down), Minimise);
        assert_eq!(k(Z::None, Key::Up), To(Z::Zoom));
        assert_eq!(k(Z::None, Key::Down), Minimise);
        // Zoomed (the zoom button, a drag to the top, or Super+Up): Down restores.
        assert_eq!(key_step(Z::None, Key::Down, false, true), Restore);
        assert_eq!(key_step(Z::None, Key::Up, false, true), Stay);
        assert_eq!(key_step(Z::None, Key::Left, false, true), To(Z::Left));
        // A full walk round the screen comes back to a half.
        let mut z = Z::None;
        for key in [Key::Left, Key::Up, Key::Right, Key::Down] {
            if let To(n) = k(z, key) {
                z = n;
            }
        }
        assert_eq!(z, Z::Right);
    }

    #[test]
    fn tiled_windows_move_through_the_tiling() {
        assert_eq!(
            key_step(Zone::None, Key::Left, true, false),
            Step::MoveTiled(Key::Left)
        );
        assert_eq!(
            key_step(Zone::None, Key::Right, true, false),
            Step::MoveTiled(Key::Right)
        );
        assert_eq!(key_step(Zone::None, Key::Up, true, false), Step::ZoomTiled);
        assert_eq!(key_step(Zone::None, Key::Down, true, true), Step::ZoomTiled);
        assert_eq!(key_step(Zone::None, Key::Down, true, false), Step::Stay);
    }

    #[test]
    fn a_dragged_snapped_window_gets_its_size_back_under_the_pointer() {
        let snapped = Rect {
            x: 12,
            y: 40,
            w: 622,
            h: 684,
        };
        // Grabbed at the middle of the bar: the 800 px wide window is centred on the pointer.
        assert_eq!(unsnap_at(snapped, (800, 500), 323, 50), (323 - 400, 40));
        // Grabbed at the left end: the pointer stays at the left end.
        assert_eq!(unsnap_at(snapped, (800, 500), 12, 50), (12, 40));
        // Grabbed past the right end (rounding): clamped to the right end.
        assert_eq!(unsnap_at(snapped, (800, 500), 700, 50), (700 - 800, 40));
        assert!(still_at(snapped, Rect { x: 13, ..snapped }));
        assert!(!still_at(snapped, Rect { w: 600, ..snapped }));
    }

    /// The C ABI runs inside the compositor: extreme boxes and pointers never overflow (a debug
    /// panic there aborts it); a loop of odd values stands in for a fuzzer.
    #[test]
    fn extreme_values_never_overflow() {
        let ex = [
            i32::MIN,
            i32::MIN + 1,
            -1,
            0,
            1,
            5,
            96,
            i32::MAX - 1,
            i32::MAX,
        ];
        for &x in &ex {
            for &w in &ex {
                let r = Rect { x, y: x, w, h: w };
                for &p in &ex {
                    let _ = zone_at(r, 15, p, x);
                    let _ = zone_at(r, 15, x, p);
                    let _ = unsnap_at(r, (w, p), p, x);
                    let _ = unsnap_at(r, (p, w), x, p);
                    let _ = still_at(
                        r,
                        Rect {
                            x: p,
                            y: p,
                            w: p,
                            h: p,
                        },
                    );
                    for z in Zone::ALL {
                        for &g in &[i32::MIN, -1, 0, 6, i32::MAX] {
                            let _ = zone_box(z, r, g, g);
                            let _ = zone_box(z, r, g, 6);
                        }
                    }
                }
            }
        }
        // A sane box in an extreme area still lands inside it where it can.
        let big = Rect {
            x: i32::MAX - 100,
            y: 0,
            w: 1000,
            h: 800,
        };
        assert!(zone_box(Zone::Left, big, 12, 6).is_some());
    }

    #[test]
    fn names_and_codes_round_trip() {
        for z in Zone::ALL {
            assert_eq!(Zone::from_name(z.name()), Some(z));
            assert_eq!(Zone::from_code(z as i32), Some(z));
        }
        assert_eq!(Zone::from_code(8), None);
        assert_eq!(Key::from_name("l"), Some(Key::Left));
        assert_eq!(Key::from_name("down"), Some(Key::Down));
        assert_eq!(Key::from_name("x"), None);
        assert_eq!(LAYOUTS.len(), 6);
    }
}
