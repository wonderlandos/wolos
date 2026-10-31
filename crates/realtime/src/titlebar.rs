//! macOS title bar geometry: where the traffic lights are and what a point hits. Pure, no rendering.

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

/// Top-left corner of each button, relative to the title bar.
pub fn button_origin(b: Button) -> (i32, i32) {
    let i = match b {
        Button::Close => 0,
        Button::Minimize => 1,
        Button::Zoom => 2,
    };
    (INSET + i * (BUTTON + GAP), (HEIGHT - BUTTON) / 2)
}

/// What a point (relative to the title bar's top-left) hits, if anything.
pub fn hit_test(width: i32, x: i32, y: i32) -> Option<Hit> {
    if x < 0 || y < 0 || x >= width || y >= HEIGHT {
        return None;
    }
    for b in [Button::Close, Button::Minimize, Button::Zoom] {
        let (bx, by) = button_origin(b);
        // Round buttons: a hit is within the circle (with 2 px of slack, like macOS).
        let (cx, cy) = (bx + BUTTON / 2, by + BUTTON / 2);
        let r = BUTTON / 2 + 2;
        if (x - cx).pow(2) + (y - cy).pow(2) <= r * r {
            return Some(Hit::Button(b));
        }
    }
    Some(Hit::Bar)
}

/// How far a light's hit circle reaches from its centre (2 px of slack, like macOS).
const REACH: i32 = BUTTON / 2 + 2;
const LIGHTS: [Button; 3] = [Button::Close, Button::Minimize, Button::Zoom];

/// The light under a point, among the first `count` lights (a fixed-size dialog shows only close).
pub fn light_at(count: i32, x: i32, y: i32) -> Option<Button> {
    match hit_test(i32::MAX, x, y)? {
        Hit::Button(b) if LIGHTS.iter().take(count.max(0) as usize).any(|l| *l == b) => Some(b),
        _ => None,
    }
}

/// Whether a point is over the group of `count` lights, the gaps between them included: sliding
/// from one light to the next must not blink the glyphs.
pub fn over_lights(count: i32, x: i32, y: i32) -> bool {
    let Some(last) = LIGHTS.get((count - 1).clamp(-1, 2) as usize) else {
        return false;
    };
    let centre = |b| button_origin(b).0 + BUTTON / 2;
    let cy = HEIGHT / 2;
    x >= centre(Button::Close) - REACH
        && x <= centre(*last) + REACH
        && y >= cy - REACH
        && y <= cy + REACH
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
    fn buttons_are_macos_spaced() {
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
