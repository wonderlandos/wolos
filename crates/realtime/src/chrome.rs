// SPDX-License-Identifier: GPL-2.0-or-later
//! Which windows get the WolOS window buttons (traffic lights) drawn by the compositor.
//! Windows that draw their own controls keep them, styled as the same lights by the theme: GTK
//! header bars, Firefox and Helium (`wol theme`), Kit windows (`button.light`). Windows with no
//! controls of their own get ours in a title strip above their content, so there are never two
//! sets and never none.

/// How the client answered xdg-decoration (or the X11 equivalent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoration {
    /// No xdg-decoration object: GTK 3/4 and libadwaita. Their header bar has the themed lights.
    None,
    /// Asked for (or accepted) server-side decorations: Qt, foot, Electron with a frame.
    Server,
    /// Explicitly asked for client-side: the app draws its own frame (Helium's custom frame,
    /// Electron `frame: false`). It keeps its buttons.
    Client,
    /// Xwayland window that wants decorations.
    X11,
    /// Xwayland menu, tooltip, splash or override-redirect window.
    X11Bare,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts<'a> {
    pub class: &'a str,
    pub decoration: Decoration,
    /// Has a parent toplevel (dialog, transient).
    pub transient: bool,
    /// Minimum size equals maximum size in either direction.
    pub fixed_size: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Lights {
    /// No strip: the window draws its own controls or is not a normal window.
    None = 0,
    /// Close only: a fixed-size dialog (macOS sheets and panels cannot be minimised or zoomed).
    Close = 1,
    /// Close, minimise, zoom.
    All = 3,
}

/// Apps that draw their own window buttons whatever the system asks (checked on the live
/// session); they keep theirs and get no strip. Window class, reason.
pub const FORCES_OWN: &[(&str, &str)] = &[
    // Steam's client draws its own CEF title bar over Xwayland and sets no decoration hints.
    ("steam", "Steam client: own CEF title bar"),
];

pub fn forces_own(class: &str) -> bool {
    FORCES_OWN
        .iter()
        .any(|(c, _)| c.eq_ignore_ascii_case(class))
}

pub fn lights(f: Facts) -> Lights {
    match f.decoration {
        Decoration::None | Decoration::Client | Decoration::X11Bare => return Lights::None,
        Decoration::Server | Decoration::X11 => {}
    }
    if forces_own(f.class) {
        Lights::None
    } else if f.transient && f.fixed_size {
        Lights::Close
    } else {
        Lights::All
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(class: &str, decoration: Decoration) -> Facts<'_> {
        Facts {
            class,
            decoration,
            transient: false,
            fixed_size: false,
        }
    }

    /// Every app WolOS ships (desktop/packages.txt, Dock pins, mimeapps defaults, wol-shell apps)
    /// and how it decorates. `Lights::None` here means its own themed lights, not no lights.
    const SHIPPED: &[(&str, Decoration, Lights)] = &[
        ("helium", Decoration::Client, Lights::None),
        ("org.gnome.Nautilus", Decoration::None, Lights::None),
        ("org.gnome.TextEditor", Decoration::None, Lights::None),
        ("org.gnome.Calculator", Decoration::None, Lights::None),
        ("io.bassi.Amberol", Decoration::None, Lights::None),
        ("org.wolos.settings", Decoration::None, Lights::None),
        ("org.wolos.store", Decoration::None, Lights::None),
        ("foot", Decoration::Server, Lights::All),
    ];

    #[test]
    fn every_shipped_app_has_exactly_one_set_of_lights() {
        for &(class, d, want) in SHIPPED {
            assert_eq!(lights(app(class, d)), want, "{class}");
        }
    }

    #[test]
    fn a_window_with_no_controls_of_its_own_gets_ours() {
        for d in [Decoration::Server, Decoration::X11] {
            assert_eq!(lights(app("anything", d)), Lights::All, "{d:?}");
        }
    }

    #[test]
    fn a_window_that_keeps_its_own_frame_gets_none() {
        // GTK and Kit: a strip above the header bar would show a second set.
        assert_eq!(
            lights(app("org.wolos.settings", Decoration::None)),
            Lights::None
        );
        assert_eq!(lights(app("code", Decoration::Client)), Lights::None);
        assert_eq!(lights(app("xmenu", Decoration::X11Bare)), Lights::None);
        assert_eq!(lights(app("Steam", Decoration::X11)), Lights::None);
    }

    #[test]
    fn fixed_size_dialogs_keep_only_close() {
        let mut f = app("xterm", Decoration::X11);
        f.transient = true;
        assert_eq!(lights(f), Lights::All, "a resizable dialog is a window");
        f.fixed_size = true;
        assert_eq!(lights(f), Lights::Close);
        f.transient = false;
        assert_eq!(
            lights(f),
            Lights::All,
            "a fixed-size main window still minimises"
        );
    }

    #[test]
    fn allowlist_entries_say_why() {
        for (class, why) in FORCES_OWN {
            assert!(!class.is_empty() && !why.is_empty());
        }
    }
}
