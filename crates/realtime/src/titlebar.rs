// SPDX-License-Identifier: GPL-2.0-or-later
//! WolOS title bar geometry: where the window buttons are and what a point hits. Pure, no
//! rendering. Spacing is the Kit's window controls (`button.light` in kit.css, `LIGHT_*` in
//! `wol_core::theme::apps`): 12 px circles, 22 px centre to centre, the first centre 22.5 px in
//! from the edge the row is aligned to.

pub const HEIGHT: i32 = 28;
pub const BUTTON: i32 = 12;
pub const GAP: i32 = 10;
pub const INSET: i32 = 17;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Close,
    Minimize,
    Zoom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Button(Button),
    /// The rest of the bar: drag to move, double-click to zoom.
    Bar,
}

/// Which edge of the bar the row of buttons starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Close leftmost, then minimise, zoom (the original ABI: `wol_lights_at`, `button_origin`).
    Left,
    /// The WolOS default: close rightmost, then zoom, minimise leftward (`wol_lights_at_w`).
    Right,
}

impl Align {
    /// The buttons by slot, slot 0 at the aligned edge. A window that shows `count` buttons shows
    /// the first `count` slots, so a close-only dialog keeps close at the edge.
    pub fn order(self) -> [Button; 3] {
        match self {
            Align::Left => [Button::Close, Button::Minimize, Button::Zoom],
            Align::Right => [Button::Close, Button::Zoom, Button::Minimize],
        }
    }

    fn slot(self, b: Button) -> i32 {
        match (self, b) {
            (_, Button::Close) => 0,
            (Align::Left, Button::Minimize) | (Align::Right, Button::Zoom) => 1,
            (Align::Left, Button::Zoom) | (Align::Right, Button::Minimize) => 2,
        }
    }
}

/// Top-left corner of each button in a bar `width` px wide, relative to the bar.
pub fn origin(align: Align, width: i32, b: Button) -> (i32, i32) {
    let step = align.slot(b) * (BUTTON + GAP);
    let x = match align {
        Align::Left => INSET + step,
        Align::Right => width - INSET - BUTTON - step,
    };
    (x, (HEIGHT - BUTTON) / 2)
}

/// Top-left corner of each button of a left-aligned row, relative to the title bar.
pub fn button_origin(b: Button) -> (i32, i32) {
    origin(Align::Left, 0, b)
}

/// How far a button's hit circle reaches from its centre (2 px of slack).
const REACH: i32 = BUTTON / 2 + 2;

fn centre(align: Align, width: i32, b: Button) -> (i32, i32) {
    let (bx, by) = origin(align, width, b);
    (bx + BUTTON / 2, by + BUTTON / 2)
}

/// The first `count` buttons of a row (`count` clamped to 0..=3).
fn shown(align: Align, count: i32) -> impl Iterator<Item = Button> {
    let n = usize::try_from(count.clamp(0, 3)).unwrap_or(0);
    align.order().into_iter().take(n)
}

/// The button whose circle (with its slack) holds the point, among the first `count`.
fn button_under(align: Align, width: i32, count: i32, x: i32, y: i32) -> Option<Button> {
    shown(align, count).find(|&b| {
        let (cx, cy) = centre(align, width, b);
        (x - cx).pow(2) + (y - cy).pow(2) <= REACH * REACH
    })
}

/// What a point (relative to the bar's top-left) hits in a bar `width` px wide with all three
/// buttons, if anything.
pub fn hit_test_in(align: Align, width: i32, x: i32, y: i32) -> Option<Hit> {
    if x < 0 || y < 0 || x >= width || y >= HEIGHT {
        return None;
    }
    Some(button_under(align, width, 3, x, y).map_or(Hit::Bar, Hit::Button))
}

/// What a point (relative to the title bar's top-left) hits on a left-aligned bar, if anything.
pub fn hit_test(width: i32, x: i32, y: i32) -> Option<Hit> {
    hit_test_in(Align::Left, width, x, y)
}

/// The button under a point, among the first `count` of a row in a bar `width` px wide (a
/// fixed-size dialog shows only close). Outside the bar is no button.
pub fn button_at(align: Align, width: i32, count: i32, x: i32, y: i32) -> Option<Button> {
    if x < 0 || y < 0 || x >= width || y >= HEIGHT {
        return None;
    }
    button_under(align, width, count, x, y)
}

/// Whether a point is over the group of the first `count` buttons, the gaps between them
/// included: sliding from one button to the next must not blink the glyphs.
pub fn over_group(align: Align, width: i32, count: i32, x: i32, y: i32) -> bool {
    // No allocation: the C ABI calls this on every pointer motion.
    let Some((lo, hi)) = shown(align, count)
        .map(|b| centre(align, width, b).0)
        .fold(None, |r: Option<(i32, i32)>, c| {
            Some(r.map_or((c, c), |(lo, hi)| (lo.min(c), hi.max(c))))
        })
    else {
        return false;
    };
    let cy = HEIGHT / 2;
    x >= lo - REACH && x <= hi + REACH && y >= cy - REACH && y <= cy + REACH
}

/// The light under a point on a left-aligned bar, among the first `count` lights.
pub fn light_at(count: i32, x: i32, y: i32) -> Option<Button> {
    button_at(Align::Left, i32::MAX, count, x, y)
}

/// Whether a point is over the group of `count` lights of a left-aligned bar.
pub fn over_lights(count: i32, x: i32, y: i32) -> bool {
    over_group(Align::Left, i32::MAX, count, x, y)
}

/// Hover state of one title bar. The bar samples it whenever the pointer or the window moves and
/// redraws when `glyphs` changes, never as a side effect of drawing: a frame only happens if
/// something asked for it, and with a hardware cursor pointer motion alone asks for none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hover {
    over: bool,
    held: bool,
}

impl Hover {
    /// `over`: the pointer is over the lights and nothing covers them. `held`: a press that began
    /// on a light is still down.
    pub fn next(self, over: bool, held: bool) -> Self {
        Self { over, held }
    }

    /// All three show together: there is no per-button state to get out of step.
    pub fn glyphs(self) -> bool {
        self.over || self.held
    }

    pub fn bits(self) -> u32 {
        self.over as u32 | (self.held as u32) << 1
    }

    pub fn from_bits(bits: u32) -> Self {
        Self {
            over: bits & 1 != 0,
            held: bits & 2 != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_are_kit_spaced() {
        assert_eq!(button_origin(Button::Close), (17, 8));
        assert_eq!(button_origin(Button::Minimize), (39, 8));
        assert_eq!(button_origin(Button::Zoom), (61, 8));
    }

    #[test]
    fn hits_buttons_bar_and_nothing() {
        assert_eq!(hit_test(600, 23, 14), Some(Hit::Button(Button::Close)));
        assert_eq!(hit_test(600, 45, 14), Some(Hit::Button(Button::Minimize)));
        assert_eq!(hit_test(600, 67, 14), Some(Hit::Button(Button::Zoom)));
        assert_eq!(hit_test(600, 300, 14), Some(Hit::Bar));
        assert_eq!(hit_test(600, 300, 40), None);
        assert_eq!(hit_test(600, 700, 10), None);
    }

    #[test]
    fn gap_between_buttons_is_bar_not_button() {
        // x = 33 is between close (17..29) and minimize (39..51), outside both circles' slack.
        assert_eq!(hit_test(600, 33, 2), Some(Hit::Bar));
    }

    /// Points on the three lights, the gaps between them, and just outside the group.
    const ON: [(i32, i32); 5] = [(23, 14), (34, 14), (45, 14), (56, 14), (67, 14)];
    const OFF: [(i32, i32); 6] = [(10, 14), (80, 14), (300, 14), (45, 3), (45, 25), (45, 60)];

    #[test]
    fn the_group_covers_all_three_lights_and_their_gaps() {
        for (x, y) in ON {
            assert!(over_lights(3, x, y), "({x}, {y})");
        }
        for (x, y) in OFF {
            assert!(!over_lights(3, x, y), "({x}, {y})");
        }
        // Every column from the left of close to the right of zoom, no holes.
        for x in 15..=75 {
            assert!(over_lights(3, x, 14), "x = {x}");
        }
        assert!(!over_lights(3, 14, 14) && !over_lights(3, 76, 14));
        // The slack band, top and bottom.
        assert!(over_lights(3, 45, 6) && over_lights(3, 45, 22));
        assert!(!over_lights(3, 45, 5) && !over_lights(3, 45, 23));
    }

    #[test]
    fn a_close_only_dialog_has_a_one_light_group() {
        assert!(over_lights(1, 23, 14));
        assert!(!over_lights(1, 45, 14) && !over_lights(1, 67, 14));
        assert!(!over_lights(0, 23, 14));
    }

    #[test]
    fn a_press_lands_on_a_light_only_inside_its_circle() {
        assert_eq!(light_at(3, 23, 14), Some(Button::Close));
        assert_eq!(light_at(3, 45, 14), Some(Button::Minimize));
        assert_eq!(light_at(3, 67, 14), Some(Button::Zoom));
        assert_eq!(light_at(3, 34, 14), None, "the gap drags the window");
        assert_eq!(light_at(3, 80, 14), None, "right of zoom is bar, not zoom");
        assert_eq!(light_at(1, 45, 14), None, "a hidden light takes no press");
        assert_eq!(light_at(0, 23, 14), None);
    }

    // Right-aligned row in a 600 px bar: close at 571..583 (centre 577, 22.5 px in from the
    // right edge counting the half pixel), zoom centre 555, minimise centre 533.
    const W: i32 = 600;

    #[test]
    fn a_right_row_puts_close_at_the_edge_then_zoom_then_minimise() {
        assert_eq!(origin(Align::Right, W, Button::Close), (571, 8));
        assert_eq!(origin(Align::Right, W, Button::Zoom), (549, 8));
        assert_eq!(origin(Align::Right, W, Button::Minimize), (527, 8));
        // Same pitch and inset as the left row, mirrored.
        for b in Align::Left.order() {
            let (l, _) = origin(Align::Left, W, b);
            let mirrored = W - l - BUTTON;
            let slot = Align::Left.order().iter().position(|o| *o == b).unwrap();
            assert_eq!(
                origin(Align::Right, W, Align::Right.order()[slot]).0,
                mirrored
            );
        }
        // The left row is the old ABI unchanged.
        for b in Align::Left.order() {
            assert_eq!(origin(Align::Left, W, b), button_origin(b));
        }
    }

    #[test]
    fn slots_follow_the_order() {
        for a in [Align::Left, Align::Right] {
            for (i, b) in a.order().into_iter().enumerate() {
                assert_eq!(usize::try_from(a.slot(b)).ok(), Some(i), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn a_right_row_hits_follow_the_bar_width() {
        assert_eq!(button_at(Align::Right, W, 3, 577, 14), Some(Button::Close));
        assert_eq!(button_at(Align::Right, W, 3, 555, 14), Some(Button::Zoom));
        assert_eq!(
            button_at(Align::Right, W, 3, 533, 14),
            Some(Button::Minimize)
        );
        assert_eq!(
            button_at(Align::Right, W, 3, 566, 14),
            None,
            "the gap drags the window"
        );
        assert_eq!(
            button_at(Align::Right, W, 3, 23, 14),
            None,
            "nothing on the left"
        );
        assert_eq!(
            button_at(Align::Right, 800, 3, 777, 14),
            Some(Button::Close)
        );
        assert_eq!(
            button_at(Align::Right, 800, 3, 577, 14),
            None,
            "the row moves with the width"
        );
        assert_eq!(
            button_at(Align::Right, W, 3, 600, 14),
            None,
            "outside the bar"
        );
        assert_eq!(hit_test_in(Align::Right, W, 300, 14), Some(Hit::Bar));
        assert_eq!(
            hit_test_in(Align::Right, W, 555, 14),
            Some(Hit::Button(Button::Zoom))
        );
        assert_eq!(hit_test_in(Align::Right, W, 555, 40), None);
    }

    #[test]
    fn a_close_only_dialog_keeps_close_at_the_right_edge() {
        assert_eq!(button_at(Align::Right, W, 1, 577, 14), Some(Button::Close));
        assert_eq!(
            button_at(Align::Right, W, 1, 555, 14),
            None,
            "a hidden button takes no press"
        );
        assert_eq!(button_at(Align::Right, W, 0, 577, 14), None);
        assert!(over_group(Align::Right, W, 1, 577, 14));
        assert!(!over_group(Align::Right, W, 1, 555, 14));
        assert!(!over_group(Align::Right, W, 0, 577, 14));
    }

    #[test]
    fn the_right_group_covers_all_three_and_their_gaps() {
        // From the left slack of minimise (533 - 8) to the right slack of close (577 + 8).
        for x in 525..=585 {
            assert!(over_group(Align::Right, W, 3, x, 14), "x = {x}");
        }
        assert!(
            !over_group(Align::Right, W, 3, 524, 14) && !over_group(Align::Right, W, 3, 586, 14)
        );
        assert!(over_group(Align::Right, W, 3, 555, 6) && over_group(Align::Right, W, 3, 555, 22));
        assert!(
            !over_group(Align::Right, W, 3, 555, 5) && !over_group(Align::Right, W, 3, 555, 23)
        );
        assert!(
            !over_group(Align::Right, W, 3, 45, 14),
            "the left row's place is bar"
        );
    }

    /// One step: pointer over the lights?, press held?, then the glyphs we must see.
    fn replay(steps: &[(bool, bool, bool)]) {
        let mut h = Hover::default();
        for (i, &(over, held, want)) in steps.iter().enumerate() {
            h = h.next(over, held);
            assert_eq!(h.glyphs(), want, "step {i}: {steps:?}");
            assert_eq!(Hover::from_bits(h.bits()), h, "the C side keeps the bits");
        }
    }

    #[test]
    fn one_sample_is_enough_to_show_and_to_hide() {
        // Live: the pointer stopped on a light after a single motion and no glyphs came.
        replay(&[(true, false, true)]);
        replay(&[(true, false, true), (false, false, false)]);
    }

    #[test]
    fn a_covered_or_moved_bar_loses_the_glyphs_without_pointer_motion() {
        // The pointer is still; the window moved, another window or a menu covered the lights,
        // or the workspace switched: the next sample says "not over".
        replay(&[
            (true, false, true),
            (false, false, false),
            (true, false, true),
        ]);
    }

    #[test]
    fn a_press_keeps_the_glyphs_until_release_wherever_the_pointer_goes() {
        replay(&[
            (true, false, true),
            (true, true, true),
            (false, true, true),
            (false, false, false),
        ]);
        replay(&[(true, true, true), (true, false, true)]);
    }

    #[test]
    fn idle_stays_idle() {
        let h = Hover::default().next(false, false);
        assert_eq!(h, Hover::default(), "no change, no redraw");
    }

    #[test]
    fn hover_bits_round_trip() {
        for over in [false, true] {
            for held in [false, true] {
                let h = Hover::default().next(over, held);
                assert_eq!(Hover::from_bits(h.bits()), h);
            }
        }
    }
}
