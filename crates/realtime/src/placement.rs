// SPDX-License-Identifier: GPL-2.0-or-later
//! New-window placement (the WolOS rule): a fresh toplevel fits the usable area (below the menu bar,
//! above the dock, inside a margin), is centred, or cascaded from the previous window of the same
//! app, and is never placed off screen. Positions are the client area; the title bar sits above it.

/// (x, y, w, h) in logical pixels.
pub type Rect = (i32, i32, i32, i32);

/// Gap kept between a new window and the screen edges, menu bar and dock.
pub const MARGIN: i32 = 8;
/// Offset of a new window from the previous window of the same app.
pub const CASCADE: i32 = 24;

/// Largest coordinate or size taken from a client or the compositor. Every input is clamped to
/// this first, so the sums below cannot overflow `i32` whatever a client puts in its xdg
/// min/max size (audit A20).
pub const MAX_COORD: i32 = 1 << 20;

fn c(v: i32) -> i32 {
    v.clamp(-MAX_COORD, MAX_COORD)
}

fn d(v: i32) -> i32 {
    v.clamp(0, MAX_COORD)
}

fn rect(r: Rect) -> Rect {
    (c(r.0), c(r.1), d(r.2), d(r.3))
}

/// Usable area: the non-exclusive `zone` (menu bar removed), cut above `dock_top` when the dock
/// overlaps it, inset by `MARGIN`.
pub fn usable(zone: Rect, dock_top: Option<i32>) -> Rect {
    let (x, y, w, mut h) = rect(zone);
    let dock_top = dock_top.map(c);
    if let Some(top) = dock_top
        && top > y
        && top < y + h
    {
        h = top - y;
    }
    (
        x + MARGIN,
        y + MARGIN,
        (w - 2 * MARGIN).max(1),
        (h - 2 * MARGIN).max(1),
    )
}

/// Client size for a new window: its `wish` clamped to `area` (title bar `bar` included) and to
/// xdg max size; xdg min size wins over the area. 0 in `min`/`max` means unset.
pub fn fit(wish: (i32, i32), min: (i32, i32), max: (i32, i32), area: Rect, bar: i32) -> (i32, i32) {
    let (area, bar) = (rect(area), d(bar));
    let axis = |wish: i32, min: i32, max: i32, room: i32| {
        let (wish, min, max) = (d(wish), d(min), d(max));
        let mut v = wish.min(room);
        if max > 0 {
            v = v.min(max);
        }
        v.max(min).max(1)
    };
    (
        axis(wish.0, min.0, max.0, area.2),
        axis(wish.1, min.1, max.1, area.3 - bar),
    )
}

/// Client-area position that keeps the frame of a window of `size` inside `area`: the title bar
/// (`bar` above the client area) never leaves the top, the top-left wins when it is too big.
pub fn clamp(size: (i32, i32), area: Rect, bar: i32, at: (i32, i32)) -> (i32, i32) {
    let (ax, ay, aw, ah) = rect(area);
    let (size, bar, at) = ((d(size.0), d(size.1)), d(bar), (c(at.0), c(at.1)));
    let (w, h) = (size.0, size.1 + bar); // frame: title bar + content
    let (x, y) = (at.0, at.1 - bar); // frame top-left
    let x = x.min(ax + aw - w).max(ax);
    let y = y.min(ay + ah - h).max(ay);
    (x, y + bar)
}

/// Client-area position for a new window of `size`. `prev`: client position of the previous
/// window of the same app (cascade); otherwise centred, a little above centre.
pub fn place(size: (i32, i32), area: Rect, bar: i32, prev: Option<(i32, i32)>) -> (i32, i32) {
    let area = rect(area);
    let (ax, ay, aw, ah) = area;
    let (size, bar, prev) = (
        (d(size.0), d(size.1)),
        d(bar),
        prev.map(|(x, y)| (c(x), c(y))),
    );
    let (w, h) = (size.0, size.1 + bar); // frame: title bar + content
    let fits = |x: i32, y: i32| x + w <= ax + aw && y + h <= ay + ah;
    let (x, y) = match prev {
        Some((px, py)) => {
            let (x, y) = (px + CASCADE, py - bar + CASCADE);
            if fits(x, y) { (x, y) } else { (ax, ay) } // ran off the area: restart top-left
        }
        None => (ax + (aw - w) / 2, ay + (ah - h) / 3),
    };
    // Never off screen: keep the frame inside the area, top-left wins when it is too big.
    clamp(size, area, bar, (x, y + bar))
}

/// Client-area position of a dialog of `size` over its parent's client rect `parent` (a
/// transient window opens centred on the window it belongs to), kept inside `area`.
pub fn centre_on(size: (i32, i32), parent: Rect, area: Rect, bar: i32) -> (i32, i32) {
    let (px, py, pw, ph) = rect(parent);
    let (size, bar) = ((d(size.0), d(size.1)), d(bar));
    let (w, h) = (size.0, size.1 + bar);
    let x = px + (pw - w) / 2;
    let y = py + (ph - h) / 2;
    clamp(size, area, bar, (x, y + bar))
}

/// A window that is already open after the usable area changed (display resize, Dock shown or
/// hidden): its size fits the new area ([`fit`]) and its frame is pulled back inside; a window
/// that still fits where it is does not move. `area` is a [`usable`] area: a frame inside the
/// zone it came from (the `MARGIN` around it included) counts as fitting, so a window the compositor
/// put at the zone edge (zoom restored, a drag) is not nudged inward (audit4 S5-19). Returns the
/// client (x, y, w, h).
pub fn refit(
    at: (i32, i32),
    size: (i32, i32),
    min: (i32, i32),
    max: (i32, i32),
    area: Rect,
    bar: i32,
) -> (i32, i32, i32, i32) {
    let (ax, ay, aw, ah) = rect(area);
    let zone = (ax - MARGIN, ay - MARGIN, aw + 2 * MARGIN, ah + 2 * MARGIN);
    let (sw, sh) = (d(size.0), d(size.1));
    if fit((sw, sh), min, max, zone, bar) == (sw, sh)
        && clamp((sw, sh), zone, bar, at) == (c(at.0), c(at.1))
    {
        return (c(at.0), c(at.1), sw, sh);
    }
    let (w, h) = fit(size, min, max, area, bar);
    let (x, y) = clamp((w, h), area, bar, at);
    (x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: i32 = 30;
    /// 1280x720 output, 28 px menu bar reserved.
    const ZONE: Rect = (0, 28, 1280, 692);

    #[test]
    fn hostile_sizes_do_not_overflow() {
        let big = (i32::MAX, i32::MAX);
        let low = (i32::MIN, i32::MIN);
        let (w, h) = fit(big, big, big, ZONE, i32::MAX);
        assert!(w <= MAX_COORD && h <= MAX_COORD);
        let _ = fit(
            low,
            low,
            low,
            (i32::MIN, i32::MIN, i32::MIN, i32::MIN),
            i32::MIN,
        );
        let _ = place(
            big,
            (i32::MAX, i32::MAX, i32::MAX, i32::MAX),
            i32::MAX,
            Some(big),
        );
        let _ = centre_on(
            big,
            (i32::MAX, i32::MIN, i32::MAX, i32::MAX),
            ZONE,
            i32::MAX,
        );
        let _ = refit(low, big, big, low, ZONE, i32::MAX);
        let _ = usable((i32::MAX, i32::MAX, i32::MAX, i32::MAX), Some(i32::MIN));
    }

    #[test]
    fn usable_area_is_below_the_menu_bar_above_the_dock_inside_the_margin() {
        let cases = [
            (ZONE, Some(636), (8, 36, 1264, 592)),
            (ZONE, None, (8, 36, 1264, 676)),
            (ZONE, Some(800), (8, 36, 1264, 676)), // dock off the zone: ignored
            ((0, 0, 10, 10), None, (8, 8, 1, 1)),  // never empty
        ];
        for (zone, dock, want) in cases {
            assert_eq!(usable(zone, dock), want, "{zone:?} {dock:?}");
        }
    }

    #[test]
    fn size_fits_the_area_respecting_min_and_max() {
        let area = (8, 36, 1264, 592);
        let cases = [
            ((800, 500), (0, 0), (0, 0), (800, 500)), // fits: unchanged
            ((1600, 1000), (0, 0), (0, 0), (1264, 562)), // too big: clamped (bar included)
            ((1600, 1000), (1300, 600), (0, 0), (1300, 600)), // min larger than area: min wins
            ((1000, 500), (0, 0), (900, 400), (900, 400)), // max respected
            ((0, 0), (0, 0), (0, 0), (1, 1)),         // never zero
        ];
        for (wish, min, max, want) in cases {
            assert_eq!(
                fit(wish, min, max, area, BAR),
                want,
                "{wish:?} {min:?} {max:?}"
            );
        }
    }

    #[test]
    fn placement_centres_cascades_and_stays_on_screen() {
        let area = (8, 36, 1264, 592);
        let cases = [
            // centred horizontally, a third from the top (frame = 300 + 30)
            ((400, 300), None, (440, 36 + 87 + BAR)),
            // cascade +24/+24 from the previous window of the same app
            ((400, 300), Some((440, 153)), (464, 177)),
            // cascade running off the bottom restarts at the top-left
            ((400, 300), Some((440, 36 + 262 + BAR)), (8, 36 + BAR)),
            // full-area window: top-left of the area, title bar on screen
            ((1264, 562), None, (8, 36 + BAR)),
            // larger than the area (min size): pinned top-left, never off screen
            ((1300, 600), None, (8, 36 + BAR)),
            ((1300, 600), Some((8, 66)), (8, 36 + BAR)),
        ];
        for (size, prev, want) in cases {
            assert_eq!(place(size, area, BAR, prev), want, "{size:?} {prev:?}");
        }
    }

    /// Live: a GTK file chooser opened centred on the screen, not on the window that asked for it.
    #[test]
    fn dialogs_centre_on_their_parent_and_stay_on_screen() {
        let area = (8, 36, 1264, 592);
        let parent = (300, 200, 600, 400); // client rect of the parent
        let cases = [
            // centred on the parent: frame 400x330 at (400, 235), client 30 lower
            ((400, 300), parent, (400, 235 + BAR)),
            // parent near the top edge: the dialog's title bar stops at the area top
            ((400, 300), (300, 40, 600, 100), (400, 36 + BAR)),
            // parent off the right edge: pulled back in
            (
                (400, 300),
                (1100, 200, 600, 400),
                (8 + 1264 - 400, 235 + BAR),
            ),
            // larger than the area: pinned top-left
            ((1300, 700), parent, (8, 36 + BAR)),
        ];
        for (size, parent, want) in cases {
            assert_eq!(
                centre_on(size, parent, area, BAR),
                want,
                "{size:?} {parent:?}"
            );
        }
    }

    /// Live (VirtualBox): the guest display shrank with the VM window and windows stayed where
    /// they were, half off screen; when the Dock hides, nothing moves.
    #[test]
    fn refit_keeps_fitting_windows_and_pulls_the_rest_back() {
        let area = (8, 36, 1264, 592);
        let none = (0, 0);
        let cases = [
            // fits where it is: untouched
            ((100, 100), (400, 300), (100, 100, 400, 300)),
            // too far right and down: pulled back, same size
            (
                (1000, 500),
                (400, 300),
                (8 + 1264 - 400, 36 + 592 - 330 + BAR, 400, 300),
            ),
            // title bar above the area (display grew smaller at the top): down to the top
            ((100, 10), (400, 300), (100, 36 + BAR, 400, 300)),
            // bigger than the area: shrunk to it, title bar on screen
            ((0, 0), (1600, 1000), (8, 36 + BAR, 1264, 562)),
        ];
        for (at, size, want) in cases {
            assert_eq!(
                refit(at, size, none, none, area, BAR),
                want,
                "{at:?} {size:?}"
            );
        }
        // min size wins over the area, as in `fit`
        assert_eq!(
            refit((100, 100), (1600, 1000), (1300, 600), none, area, BAR),
            (8, 36 + BAR, 1300, 600)
        );
    }

    /// Audit4 S5-19: the margin is for new windows. A window flush with the zone edge (zoom
    /// restored by the compositor, dragged there) stays where it is; one that sticks out of the zone is
    /// still pulled back inside the margin.
    #[test]
    fn refit_does_not_nudge_a_window_inside_the_zone() {
        let area = usable(ZONE, None); // (8, 36, 1264, 676)
        let none = (0, 0);
        let flush = ((0, 28 + BAR), (1280, 692 - BAR));
        assert_eq!(
            refit(flush.0, flush.1, none, none, area, BAR),
            (0, 28 + BAR, 1280, 692 - BAR),
            "the whole zone: untouched"
        );
        assert_eq!(
            refit((1280 - 400, 28 + BAR), (400, 300), none, none, area, BAR),
            (880, 58, 400, 300),
            "top-right corner of the zone: untouched"
        );
        // one pixel out of the zone: back inside the margin, as before
        assert_eq!(
            refit((1280 - 399, 28 + BAR), (400, 300), none, none, area, BAR),
            (8 + 1264 - 400, 28 + BAR + 8, 400, 300)
        );
        // taller than the zone: cut to the inset area
        assert_eq!(
            refit((0, 28 + BAR), (1280, 800), none, none, area, BAR),
            (8, 36 + BAR, 1264, 676 - BAR)
        );
        // max size still applies to a window in the zone
        assert_eq!(
            refit((100, 100), (600, 400), none, (500, 0), area, BAR),
            (100, 100, 500, 400)
        );
    }
}
