// SPDX-License-Identifier: GPL-2.0-or-later
//! Which windows get the WolOS window buttons (traffic lights) drawn by the compositor.
//! The compositor owns the lights wherever an app gives up its own: Atlas, Electron with a frame,
//! Qt, X11 and Kit windows get them in a title strip above their content. Apps that cannot give
//! theirs up keep them, styled as the same lights by the theme: GTK header bars, Firefox, frameless
//! Electron. Those get no strip, so there are never two sets and never none. Either way the
//! controls sit on the right, close at the edge ([`drawn`], [`STRIP_ALIGN`]). Wolkit's native
//! windows ask for server-side decorations, so they get the strip, glass ones included.

/// How the client answered xdg-decoration (or the X11 equivalent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoration {
    /// No xdg-decoration object: GTK 3/4 and libadwaita. Their header bar has the themed lights.
    None,
    /// Asked for (or accepted) server-side decorations: Qt, foot, Electron with a frame.
    Server,
    /// Explicitly asked for client-side: the app draws its own frame (Electron `frame: false`).
    /// It keeps its buttons.
    Client,
    /// Xwayland window that wants decorations.
    X11,
    /// Xwayland menu, tooltip, splash or override-redirect window.
    X11Bare,
    /// A window of a `wol-*` program: Kit draws no window buttons. GTK has no way to ask for
    /// server-side decorations and hosted apps keep their own app id, so the program tells.
    Kit,
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
    /// Close only: a fixed-size dialog (sheets and panels cannot be minimised or zoomed).
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
    let class = crate::glass::base_class(class);
    FORCES_OWN
        .iter()
        .any(|(c, _)| c.eq_ignore_ascii_case(class))
}

/// Who draws a window's controls. Every one of them is right-aligned, close at the edge, then
/// zoom and minimise (decision 2026-11-03): the compositor strip by [`STRIP_ALIGN`] (hyprbars
/// `bar_buttons_alignment = "right"`), the themed clients by their button layout
/// (`wol_core::theme::apps::BUTTON_LAYOUT` `:minimize,maximize,close`, the Firefox sheet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drawn {
    /// The compositor's title strip (`lights` is not `None`).
    Strip,
    /// The app's own controls in its header bar, styled by the WolOS theme (GTK 3/4, Firefox,
    /// the engine's GTK frame).
    Theme,
    /// The app's own controls the theme cannot reach (frameless Electron, [`FORCES_OWN`]), or no
    /// controls at all (X11 menus and tooltips).
    App,
}

/// Where the compositor strip's buttons start from.
pub const STRIP_ALIGN: crate::titlebar::Align = crate::titlebar::Align::Right;

pub fn drawn(f: Facts) -> Drawn {
    match (lights(f), f.decoration) {
        (Lights::Close | Lights::All, _) => Drawn::Strip,
        (Lights::None, Decoration::None) => Drawn::Theme,
        (Lights::None, _) => Drawn::App,
    }
}

pub fn lights(f: Facts) -> Lights {
    match f.decoration {
        Decoration::None | Decoration::Client | Decoration::X11Bare => return Lights::None,
        Decoration::Server | Decoration::X11 | Decoration::Kit => {}
    }
    if forces_own(f.class) {
        Lights::None
    } else if f.fixed_size && (f.transient || f.decoration == Decoration::Kit) {
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

    /// Every app WolOS ships (wolos-iso/profile/packages.desktop, Dock pins, mimeapps defaults, wol-shell apps)
    /// and how it decorates. `Lights::None` here means its own themed lights, not no lights.
    const SHIPPED: &[(&str, Decoration, Lights)] = &[
        ("atlas", Decoration::Server, Lights::All),
        ("org.gnome.Nautilus", Decoration::None, Lights::None),
        ("org.gnome.TextEditor", Decoration::None, Lights::None),
        ("org.gnome.Calculator", Decoration::None, Lights::None),
        ("io.bassi.Amberol", Decoration::None, Lights::None),
        ("org.wolos.settings", Decoration::Kit, Lights::All),
        ("org.wolos.store", Decoration::Kit, Lights::All),
        ("foot", Decoration::Server, Lights::All),
    ];

    #[test]
    fn every_shipped_app_has_exactly_one_set_of_lights() {
        for &(class, d, want) in SHIPPED {
            assert_eq!(lights(app(class, d)), want, "{class}");
        }
    }

    #[test]
    fn a_fixed_size_kit_window_only_closes() {
        let f = Facts {
            fixed_size: true,
            ..app("org.wolos.flatpak-consent", Decoration::Kit)
        };
        assert_eq!(lights(f), Lights::Close);
    }

    #[test]
    fn a_window_with_no_controls_of_its_own_gets_ours() {
        for d in [Decoration::Server, Decoration::X11, Decoration::Kit] {
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

    #[test]
    fn every_toplevel_kind_has_its_controls_drawn_by_one_side() {
        use Decoration::*;
        for (d, want) in [
            (None, Drawn::Theme),
            (Server, Drawn::Strip),
            (Client, Drawn::App),
            (X11, Drawn::Strip),
            (X11Bare, Drawn::App),
            (Kit, Drawn::Strip),
        ] {
            assert_eq!(drawn(app("anything", d)), want, "{d:?}");
            assert_eq!(
                drawn(app("anything", d)) == Drawn::Strip,
                lights(app("anything", d)) != Lights::None,
                "{d:?}: the strip exactly when it has lights"
            );
        }
        assert_eq!(drawn(app("steam", X11)), Drawn::App);
        assert_eq!(STRIP_ALIGN, crate::titlebar::Align::Right);
        assert_eq!(
            STRIP_ALIGN.order()[0],
            crate::titlebar::Button::Close,
            "close at the edge"
        );
    }

    #[test]
    fn a_glass_window_keeps_its_app_s_controls() {
        // The glass suffix changes nothing about the controls: the app is the same app.
        for (class, d, want) in SHIPPED {
            let glass = format!("{class}{}", crate::glass::APP_SUFFIX);
            assert_eq!(lights(app(&glass, *d)), *want, "{glass}");
        }
        assert!(forces_own("steam.glass") && !forces_own("steam.glassy"));
        let wolkit = app("org.wolos.todos.glass", Decoration::Server);
        assert_eq!((lights(wolkit), drawn(wolkit)), (Lights::All, Drawn::Strip));
    }
}
