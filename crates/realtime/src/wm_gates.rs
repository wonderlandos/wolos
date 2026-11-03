// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-hypr input gates (audit4 S5-2, S5-3, S5-11): which new windows the plugin places, only
//! clamps or leaves alone, which axes of a window are fixed, and what each watched key sends to
//! the shell socket. Pure; the plugin reads the compositor state and calls these through
//! `ffi.rs` (`wol_x11_placement`, `wol_fixed_axes`, `wol_placement_limits`, `wol_keys_step`).

/// What `onOpenLate` does with a new floating window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Not a window to place: X11 override-redirect menus, tooltips, combo lists, borderless
    /// X11 popups. They position themselves at the pointer.
    Skip = 0,
    /// An X11 window that asked for its own position (WM_NORMAL_HINTS USPosition/PPosition):
    /// kept there, only pulled back into the zone.
    ClampOnly = 1,
    /// WolOS placement: own size cut to the zone, centred on the parent or cascaded.
    Place = 2,
}

/// ICCCM WM_NORMAL_HINTS flags (xcb_icccm.h): USPosition, PPosition.
pub const X11_POSITION_HINTS: u32 = (1 << 0) | (1 << 2);

pub fn placement_for(
    x11: bool,
    override_redirect: bool,
    no_borders: bool,
    size_hint_flags: u32,
) -> Placement {
    if !x11 {
        return Placement::Place;
    }
    if override_redirect || no_borders {
        return Placement::Skip;
    }
    if size_hint_flags & X11_POSITION_HINTS != 0 {
        Placement::ClampOnly
    } else {
        Placement::Place
    }
}

/// A window the plugin may move later (cascade origin, refit after a zone change): the windows
/// `placement_for` places or clamps, never the popups it skips.
pub fn managed(x11: bool, override_redirect: bool, no_borders: bool) -> bool {
    placement_for(x11, override_redirect, no_borders, 0) != Placement::Skip
}

/// Per-axis fixed size from client limits as `wol_fit` reads them (0 = unset): an axis is fixed
/// when its min equals its max (1 px limits count as unset).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixedAxes {
    pub w: bool,
    pub h: bool,
}

impl FixedAxes {
    pub fn of(min: (i32, i32), max: (i32, i32)) -> Self {
        FixedAxes {
            w: min.0 > 1 && min.0 == max.0,
            h: min.1 > 1 && min.1 == max.1,
        }
    }
    pub fn any(self) -> bool {
        self.w || self.h
    }
    /// Bits for the C ABI: 1 width, 2 height.
    pub fn bits(self) -> u32 {
        u32::from(self.w) | (u32::from(self.h) << 1)
    }
    /// Limits for `fit`/`refit` with the fixed axes pinned to the current size: placement keeps
    /// them and may still cut the other axis (a fixed-width tall palette fits the band).
    pub fn pin(self, cur: (i32, i32), min: &mut (i32, i32), max: &mut (i32, i32)) {
        if self.w {
            (min.0, max.0) = (cur.0, cur.0);
        }
        if self.h {
            (min.1, max.1) = (cur.1, cur.1);
        }
    }
}

/// Keys the plugin watches (the caller maps evdev codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Super,
    Tab,
    Esc,
    Other,
}

impl Key {
    pub fn from_code(c: i32) -> Self {
        match c {
            0 => Key::Super,
            1 => Key::Tab,
            2 => Key::Esc,
            _ => Key::Other,
        }
    }
}

/// One key event with the state the caller read from the compositor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyEvent {
    pub pressed: bool,
    /// Super in the keyboard modifiers.
    pub super_held: bool,
    pub shift: bool,
    /// Ctrl or Alt in the keyboard modifiers.
    pub other_mods: bool,
    /// A session lock holds the screen.
    pub locked: bool,
    /// The `wol-panel-switcher` layer is on screen.
    pub switcher_mapped: bool,
    /// Any shell panel layer is on screen.
    pub shell_layer_mapped: bool,
}

pub const KEYF_PRESSED: u32 = 1;
pub const KEYF_SUPER_HELD: u32 = 2;
pub const KEYF_SHIFT: u32 = 4;
pub const KEYF_OTHER_MODS: u32 = 8;
pub const KEYF_LOCKED: u32 = 16;
pub const KEYF_SWITCHER_MAPPED: u32 = 32;
pub const KEYF_SHELL_LAYER_MAPPED: u32 = 64;

impl KeyEvent {
    pub fn from_flags(f: u32) -> Self {
        KeyEvent {
            pressed: f & KEYF_PRESSED != 0,
            super_held: f & KEYF_SUPER_HELD != 0,
            shift: f & KEYF_SHIFT != 0,
            other_mods: f & KEYF_OTHER_MODS != 0,
            locked: f & KEYF_LOCKED != 0,
            switcher_mapped: f & KEYF_SWITCHER_MAPPED != 0,
            shell_layer_mapped: f & KEYF_SHELL_LAYER_MAPPED != 0,
        }
    }
}

/// A line for `$XDG_RUNTIME_DIR/wol-shell.sock` (shell::protocol).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    None = 0,
    HideSwitcher = 1,
    ShowSwitcher = 2,
    ShowSwitcherPrev = 3,
    HideTransient = 4,
}

impl Line {
    /// NUL-terminated wire text (C ABI hands out the pointer).
    pub fn wire(self) -> &'static std::ffi::CStr {
        match self {
            Line::None => c"",
            Line::HideSwitcher => c"hide switcher\n",
            Line::ShowSwitcher => c"show switcher\n",
            Line::ShowSwitcherPrev => c"show switcher-prev\n",
            Line::HideTransient => c"hide transient\n",
        }
    }
    pub fn from_code(c: i32) -> Self {
        match c {
            1 => Line::HideSwitcher,
            2 => Line::ShowSwitcher,
            3 => Line::ShowSwitcherPrev,
            4 => Line::HideTransient,
            _ => Line::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyAction {
    pub line: Line,
    /// Cancel the event: no bind runs, no client sees it.
    pub consume: bool,
}

impl KeyAction {
    const NONE: KeyAction = KeyAction {
        line: Line::None,
        consume: false,
    };
    /// Bits for the C ABI: the line code in the low byte, 0x100 consume.
    pub fn bits(self) -> u32 {
        self.line as u32 | (u32::from(self.consume) << 8)
    }
}

/// ⌘Tab over one ordered socket (audit4 S5-3): the plugin sends `show switcher` itself on Tab
/// with Super down and consumes the key, so the conf's exec bind (the fallback without the
/// plugin) does not run as well. The Super release then always follows the show on the same
/// socket: the strip can no longer open after the release that should have closed it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwitchKeys {
    pub super_down: bool,
    /// A Tab was seen since Super went down.
    pub switching: bool,
    /// The Tab press was ours: its release is too.
    pub tab_consumed: bool,
}

impl SwitchKeys {
    pub fn from_bits(b: u32) -> Self {
        SwitchKeys {
            super_down: b & 1 != 0,
            switching: b & 2 != 0,
            tab_consumed: b & 4 != 0,
        }
    }
    pub fn bits(self) -> u32 {
        u32::from(self.super_down)
            | (u32::from(self.switching) << 1)
            | (u32::from(self.tab_consumed) << 2)
    }

    pub fn on(&mut self, key: Key, e: KeyEvent) -> KeyAction {
        match key {
            Key::Super => {
                self.super_down = e.pressed;
                let mut a = KeyAction::NONE;
                // Releasing Super finishes ⌘Tab: also while the strip waits for its client list
                // (not mapped yet), hence the Tab seen since Super went down.
                if !e.pressed && (self.switching || e.switcher_mapped) {
                    a.line = Line::HideSwitcher;
                }
                if !e.pressed {
                    self.switching = false;
                }
                a
            }
            Key::Tab if !e.pressed => {
                let ours = self.tab_consumed;
                self.tab_consumed = false;
                KeyAction {
                    line: Line::None,
                    consume: ours,
                }
            }
            Key::Tab => {
                if !(self.super_down || e.super_held) || e.other_mods || e.locked {
                    return KeyAction::NONE;
                }
                self.switching = true;
                self.tab_consumed = true;
                KeyAction {
                    line: if e.shift {
                        Line::ShowSwitcherPrev
                    } else {
                        Line::ShowSwitcher
                    },
                    consume: true,
                }
            }
            Key::Esc if e.pressed && e.shell_layer_mapped => KeyAction {
                line: Line::HideTransient,
                consume: false,
            },
            Key::Esc | Key::Other => KeyAction::NONE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S5-2: X11 menus, tooltips and combo lists are not placed; X11 windows with their own
    /// position are only clamped.
    #[test]
    fn x11_popups_are_skipped_and_positioned_windows_only_clamped() {
        use Placement::*;
        assert_eq!(placement_for(false, false, false, 0), Place); // xdg toplevel
        assert_eq!(placement_for(false, true, true, X11_POSITION_HINTS), Place); // X11 fields ignored
        assert_eq!(placement_for(true, true, false, 0), Skip); // override-redirect menu
        assert_eq!(placement_for(true, false, true, 0), Skip); // borderless X11 popup
        assert_eq!(placement_for(true, false, false, 0), Place);
        assert_eq!(placement_for(true, false, false, 1), ClampOnly); // USPosition (xterm -geometry)
        assert_eq!(placement_for(true, false, false, 4), ClampOnly); // PPosition
        assert_eq!(placement_for(true, false, false, 16), Place); // PMinSize only
        assert!(managed(false, false, false));
        assert!(managed(true, false, false));
        assert!(
            !managed(true, true, false),
            "no cascade from or refit of popups"
        );
        assert!(!managed(true, false, true));
    }

    /// S5-11: a fixed-width tall window keeps its width and may still be cut vertically.
    #[test]
    fn fixed_axes_are_per_axis() {
        assert!(!FixedAxes::of((0, 0), (0, 0)).any());
        let w = FixedAxes::of((300, 200), (300, 0));
        assert_eq!((w.w, w.h, w.bits()), (true, false, 1));
        assert_eq!(FixedAxes::of((400, 300), (400, 300)).bits(), 3);
        assert!(!FixedAxes::of((1, 1), (1, 1)).any(), "xdg 0/1: unset");
        let (mut min, mut max) = ((300, 200), (300, 0));
        w.pin((300, 2000), &mut min, &mut max);
        assert_eq!((min, max), ((300, 200), (300, 0)), "height stays free");
        let (mut min, mut max) = ((0, 0), (0, 0));
        FixedAxes { w: true, h: true }.pin((400, 300), &mut min, &mut max);
        assert_eq!((min, max), ((400, 300), (400, 300)));
    }

    fn ev(pressed: bool, super_held: bool, shift: bool) -> KeyEvent {
        KeyEvent {
            pressed,
            super_held,
            shift,
            ..Default::default()
        }
    }

    /// S5-3: the show goes over the same socket as the release, before it; the Tab is consumed.
    #[test]
    fn command_tab_shows_in_order_and_consumes_only_the_tab() {
        let mut s = SwitchKeys::default();
        assert_eq!(s.on(Key::Super, ev(true, false, false)), KeyAction::NONE);
        let a = s.on(Key::Tab, ev(true, true, false));
        assert_eq!((a.line, a.consume), (Line::ShowSwitcher, true));
        let a = s.on(Key::Tab, ev(false, true, false));
        assert_eq!(
            (a.line, a.consume),
            (Line::None, true),
            "its release is ours too"
        );
        let a = s.on(Key::Tab, ev(true, true, true));
        assert_eq!((a.line, a.consume), (Line::ShowSwitcherPrev, true));
        s.on(Key::Tab, ev(false, true, true));
        let a = s.on(Key::Super, ev(false, false, false));
        assert_eq!((a.line, a.consume), (Line::HideSwitcher, false));
        assert!(!s.switching);

        // A fast tap: show then hide, in that order; Tab released after Super still consumed.
        let mut f = SwitchKeys::default();
        f.on(Key::Super, ev(true, false, false));
        assert_eq!(
            f.on(Key::Tab, ev(true, true, false)).line,
            Line::ShowSwitcher
        );
        assert_eq!(
            f.on(Key::Super, ev(false, false, false)).line,
            Line::HideSwitcher
        );
        assert_eq!(f.on(Key::Tab, ev(false, false, false)).bits(), 0x100);

        // Plain Tab, Ctrl+Super+Tab, Tab under the lock: untouched.
        let mut p = SwitchKeys::default();
        assert_eq!(p.on(Key::Tab, ev(true, false, false)), KeyAction::NONE);
        assert_eq!(p.on(Key::Tab, ev(false, false, false)), KeyAction::NONE);
        p.on(Key::Super, ev(true, false, false));
        let ctrl = KeyEvent {
            other_mods: true,
            ..ev(true, true, false)
        };
        assert_eq!(p.on(Key::Tab, ctrl), KeyAction::NONE);
        let locked = KeyEvent {
            locked: true,
            ..ev(true, true, false)
        };
        assert_eq!(p.on(Key::Tab, locked), KeyAction::NONE);
        assert_eq!(p.on(Key::Super, ev(false, false, false)).line, Line::None);

        // Super held from before the plugin loaded (modifier state only): still a switch.
        let mut late = SwitchKeys::default();
        assert_eq!(
            late.on(Key::Tab, ev(true, true, false)).line,
            Line::ShowSwitcher
        );

        // The strip on screen, Super released: commits.
        let mut m = SwitchKeys::default();
        m.on(Key::Super, ev(true, false, false));
        let rel = KeyEvent {
            switcher_mapped: true,
            ..ev(false, false, false)
        };
        assert_eq!(m.on(Key::Super, rel).line, Line::HideSwitcher);
    }

    #[test]
    fn esc_goes_to_the_shell_only_while_a_panel_is_up_and_is_never_consumed() {
        let mut e = SwitchKeys::default();
        assert_eq!(e.on(Key::Esc, ev(true, false, false)), KeyAction::NONE);
        let esc = KeyEvent {
            shell_layer_mapped: true,
            ..ev(true, false, false)
        };
        assert_eq!(
            e.on(Key::Esc, esc),
            KeyAction {
                line: Line::HideTransient,
                consume: false
            }
        );
        assert_eq!(e.on(Key::Other, esc), KeyAction::NONE);
    }

    #[test]
    fn bits_round_trip_for_the_c_abi() {
        for b in 0..8 {
            assert_eq!(SwitchKeys::from_bits(b).bits(), b);
        }
        let e = KeyEvent::from_flags(KEYF_PRESSED | KEYF_SHIFT | KEYF_SHELL_LAYER_MAPPED);
        assert!(e.pressed && e.shift && e.shell_layer_mapped && !e.super_held && !e.locked);
        for (code, wire) in [
            (0, ""),
            (1, "hide switcher\n"),
            (2, "show switcher\n"),
            (3, "show switcher-prev\n"),
            (4, "hide transient\n"),
            (99, ""),
        ] {
            assert_eq!(Line::from_code(code).wire().to_str().unwrap(), wire);
        }
        assert_eq!(Key::from_code(1), Key::Tab);
        assert_eq!(Key::from_code(-5), Key::Other);
    }
}
