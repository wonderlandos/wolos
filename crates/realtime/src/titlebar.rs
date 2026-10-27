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

/// macOS shows the button glyphs on all three when the pointer is over any of them.
pub fn show_glyphs(hover: Option<Hit>) -> bool {
    matches!(hover, Some(Hit::Button(_)))
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

    #[test]
    fn glyphs_show_only_when_hovering_a_button() {
        assert!(show_glyphs(Some(Hit::Button(Button::Zoom))));
        assert!(!show_glyphs(Some(Hit::Bar)));
        assert!(!show_glyphs(None));
    }
}
