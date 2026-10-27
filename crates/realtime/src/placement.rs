//! New-window placement (macOS rule): a fresh toplevel fits the usable area (below the menu bar,
//! above the dock, inside a margin), is centred, or cascaded from the previous window of the same
//! app, and is never placed off screen. Positions are the client area; the title bar sits above it.

/// (x, y, w, h) in logical pixels.
pub type Rect = (i32, i32, i32, i32);

/// Gap kept between a new window and the screen edges, menu bar and dock.
pub const MARGIN: i32 = 8;
/// Offset of a new window from the previous window of the same app.
pub const CASCADE: i32 = 24;

/// Usable area: the non-exclusive `zone` (menu bar removed), cut above `dock_top` when the dock
/// overlaps it, inset by `MARGIN`.
pub fn usable(zone: Rect, dock_top: Option<i32>) -> Rect {
    let (x, y, w, mut h) = zone;
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
    let axis = |wish: i32, min: i32, max: i32, room: i32| {
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

/// Client-area position for a new window of `size`. `prev`: client position of the previous
/// window of the same app (cascade); otherwise centred, a little high like macOS.
pub fn place(size: (i32, i32), area: Rect, bar: i32, prev: Option<(i32, i32)>) -> (i32, i32) {
    let (ax, ay, aw, ah) = area;
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
    let x = x.min(ax + aw - w).max(ax);
    let y = y.min(ay + ah - h).max(ay);
    (x, y + bar)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: i32 = 30;
    /// 1280x720 output, 28 px menu bar reserved.
    const ZONE: Rect = (0, 28, 1280, 692);

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
}
