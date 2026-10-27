//! Screenshot toolbar (Super+Shift+5) model: what to capture, where it goes, the wol-screenshot argv.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Screen,
    Window,
    Selection,
    RecordScreen,
    RecordSelection,
}

impl Mode {
    pub const ALL: [Mode; 5] = [
        Mode::Screen,
        Mode::Window,
        Mode::Selection,
        Mode::RecordScreen,
        Mode::RecordSelection,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Mode::Screen => "Capture Entire Screen",
            Mode::Window => "Capture Selected Window",
            Mode::Selection => "Capture Selected Portion",
            Mode::RecordScreen => "Record Entire Screen",
            Mode::RecordSelection => "Record Selected Portion",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Mode::Screen => "wolos-capture-screen-symbolic",
            Mode::Window => "wolos-capture-window-symbolic",
            Mode::Selection => "wolos-capture-selection-symbolic",
            Mode::RecordScreen => "wolos-record-screen-symbolic",
            Mode::RecordSelection => "wolos-record-selection-symbolic",
        }
    }
    pub fn records(self) -> bool {
        matches!(self, Mode::RecordScreen | Mode::RecordSelection)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dest {
    Desktop,
    Pictures,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub dest: Dest,
    /// Seconds before capture: 0, 5 or 10.
    pub delay: u32,
}

/// Wait between hiding the toolbar and capturing, so the compositor has drawn a frame without it.
pub const SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

pub const DELAYS: [u32; 3] = [0, 5, 10];

impl Default for Options {
    fn default() -> Self {
        Options {
            dest: Dest::Desktop,
            delay: 0,
        }
    }
}

/// `wol-screenshot` argv. `window` is the focused window's "x,y wxh" (Window mode only).
pub fn argv(mode: Mode, o: Options, window: Option<&str>, home: &str) -> Vec<String> {
    let mut v = vec!["wol-screenshot".to_string()];
    match (mode, window) {
        (Mode::Screen, _) | (Mode::Window, None) => v.push("full".into()),
        (Mode::Window, Some(g)) => {
            v.push("window".into());
            v.extend(g.split(' ').map(String::from));
        }
        (Mode::Selection, _) => v.push("region".into()),
        (Mode::RecordScreen, _) => v.push("record".into()),
        (Mode::RecordSelection, _) => v.push("record-region".into()),
    }
    let dir = match (o.dest, mode.records()) {
        (Dest::Desktop, _) => format!("{home}/Desktop"),
        (Dest::Pictures, false) => format!("{home}/Pictures/Screenshots"),
        (Dest::Pictures, true) => format!("{home}/Movies"),
    };
    v.extend(["--dir".into(), dir]);
    if o.delay > 0 {
        v.extend(["--delay".into(), o.delay.to_string()]);
    }
    v
}

/// The capture button's title.
pub fn action(mode: Mode) -> &'static str {
    if mode.records() { "Record" } else { "Capture" }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Screenshots land on the Desktop, as on a Mac and as ⇧⌘3 does (wol-screenshot).
    #[test]
    fn screenshots_go_to_the_desktop_by_default() {
        assert_eq!(Options::default().dest, Dest::Desktop);
    }

    #[test]
    fn argv_per_mode() {
        let pics = Options {
            dest: Dest::Pictures,
            delay: 0,
        };
        let desk = Options {
            dest: Dest::Desktop,
            delay: 5,
        };
        for (mode, o, win, want) in [
            (
                Mode::Screen,
                pics,
                None,
                "wol-screenshot full --dir /h/Pictures/Screenshots",
            ),
            (
                Mode::Selection,
                desk,
                None,
                "wol-screenshot region --dir /h/Desktop --delay 5",
            ),
            (
                Mode::Window,
                pics,
                Some("10,20 300x200"),
                "wol-screenshot window 10,20 300x200 --dir /h/Pictures/Screenshots",
            ),
            (
                Mode::Window,
                pics,
                None,
                "wol-screenshot full --dir /h/Pictures/Screenshots",
            ),
            (
                Mode::RecordScreen,
                pics,
                None,
                "wol-screenshot record --dir /h/Movies",
            ),
            (
                Mode::RecordSelection,
                desk,
                None,
                "wol-screenshot record-region --dir /h/Desktop --delay 5",
            ),
        ] {
            assert_eq!(argv(mode, o, win, "/h").join(" "), want, "{mode:?}");
        }
    }

    /// Live Hyprland: the shot contained the toolbar (captured before the unmap was drawn).
    #[test]
    fn capture_waits_for_a_frame_without_the_toolbar() {
        assert!(SETTLE >= std::time::Duration::from_millis(2 * 1000 / 30));
    }

    #[test]
    fn action_titles() {
        assert_eq!(action(Mode::Screen), "Capture");
        assert_eq!(action(Mode::RecordSelection), "Record");
    }
}
