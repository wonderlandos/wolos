//! Dock pins, running state and the menu bar's frontmost app name (port of Iara.Core.Shell.Frontmost).
use crate::desktop::Entry;

#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub id: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub exec: &'static [&'static str],
}

/// No focused window: no app name. WolOS has no resident Finder; naming Files would claim
/// an app that is not running and offer its menu.
pub const DESKTOP_NAME: &str = "";

/// Dock Trash tile: opens the trash in Files.
pub const TRASH: &[&str] = &["nautilus", "trash:///"];

/// Default dock (WolOS design icons). Apps are mature ports (desktop/packages.txt): `id` is the app's
/// Wayland app-id, which Hyprland reports as the window class (running dot, menu bar name).
pub const PINS: &[Pin] = &[
    Pin {
        id: "org.gnome.Nautilus",
        name: "Files",
        icon: "wolos-files",
        exec: &["nautilus"],
    },
    Pin {
        id: "wol-launcher",
        name: "Launchpad",
        icon: "wolos-launcher",
        exec: &["wol-spotlight"],
    },
    Pin {
        id: "firefox",
        name: "Browser",
        icon: "wolos-browser",
        exec: &["firefox"],
    },
    Pin {
        id: "org.gnome.Geary",
        name: "Mail",
        icon: "wolos-mail",
        exec: &["geary"],
    },
    Pin {
        id: "io.bassi.Amberol",
        name: "Music",
        icon: "wolos-music",
        exec: &["amberol"],
    },
    Pin {
        id: "org.gnome.Shotwell",
        name: "Photos",
        icon: "wolos-photos",
        exec: &["shotwell"],
    },
    Pin {
        id: "org.gnome.Calendar",
        name: "Calendar",
        icon: "wolos-calendar",
        exec: &["gnome-calendar"],
    },
    Pin {
        id: "org.gnome.Gnote",
        name: "Notes",
        icon: "wolos-notes",
        exec: &["gnote"],
    },
    Pin {
        id: "foot",
        name: "Terminal",
        icon: "wolos-terminal",
        exec: &["foot"],
    },
    Pin {
        id: "org.wolos.store",
        name: "App Store",
        icon: "wolos-software",
        exec: &["wol-store"],
    },
    Pin {
        id: "org.wolos.settings",
        name: "System Settings",
        icon: "wolos-settings",
        exec: &["wol-settings"],
    },
];

fn program(exec0: &str) -> &str {
    exec0.rsplit('/').next().unwrap_or(exec0)
}

/// Window classes that belong to the pin: its id (the app-id) and its program.
fn pin_classes(p: &Pin) -> impl Iterator<Item = &str> {
    std::iter::once(p.id).chain(p.exec.first().map(|e| program(e)))
}

/// Running when a window class (lowercased by `hypr::running_classes`) is one of the pin's.
pub fn is_running(p: &Pin, classes: &std::collections::HashSet<String>) -> bool {
    pin_classes(p).any(|c| classes.contains(&c.to_lowercase()))
}

/// App-ids are mixed case (org.gnome.Nautilus); compare case-insensitively.
fn pin_for<'a>(class: &str, pins: &'a [Pin]) -> Option<&'a Pin> {
    pins.iter().find(|p| {
        pin_classes(p).any(|c| c.eq_ignore_ascii_case(class)) || p.name.eq_ignore_ascii_case(class)
    })
}

/// The pin an installed .desktop entry is (desktop id or program), so Launchpad lists it once,
/// under the WolOS name.
pub fn pin_for_entry<'a>(e: &Entry, pins: &'a [Pin]) -> Option<&'a Pin> {
    let id = e.id.trim_end_matches(".desktop");
    let prog = e.exec.first().map(|x| program(x));
    pins.iter().find(|p| {
        pin_classes(p).any(|c| c.eq_ignore_ascii_case(id) || prog.is_some_and(|x| x == c))
    })
}

fn entry_for<'a>(class: &str, installed: &'a [Entry]) -> Option<&'a Entry> {
    installed.iter().find(|e| {
        e.id.trim_end_matches(".desktop")
            .eq_ignore_ascii_case(class)
            || e.exec
                .first()
                .is_some_and(|x| program(x).eq_ignore_ascii_case(class))
    })
}

/// Pin name, else installed .desktop Name, else window title, else class; empty class = desktop.
pub fn frontmost_name(class: &str, title: &str, pins: &[Pin], installed: &[Entry]) -> String {
    if class.is_empty() && title.is_empty() {
        return DESKTOP_NAME.into();
    }
    if let Some(p) = pin_for(class, pins) {
        return p.name.into();
    }
    if let Some(e) = entry_for(class, installed) {
        return e.name.clone();
    }
    if class.is_empty() {
        title.into()
    } else {
        class.into()
    }
}

/// Pin icon, else the installed .desktop Icon, else the class itself (icon theme lookup).
pub fn icon_for(class: &str, pins: &[Pin], installed: &[Entry]) -> String {
    if let Some(p) = pin_for(class, pins) {
        return p.icon.into();
    }
    match entry_for(class, installed) {
        Some(e) if !e.icon.is_empty() => e.icon.clone(),
        _ => class.to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_window_is_named_and_marked_running() {
        // Live: the menu bar read "org.wolos.settings" (pin id was "wol-settings", not the app id).
        let class = "org.wolos.settings";
        assert_eq!(
            frontmost_name(class, "Settings", PINS, &[]),
            "System Settings"
        );
        let settings = PINS.iter().find(|p| p.exec == ["wol-settings"]).unwrap();
        assert!(is_running(settings, &[class.to_owned()].into()));
    }

    #[test]
    fn frontmost_names() {
        let installed = [Entry {
            id: "org.gnome.Calculator.desktop".into(),
            name: "Calculator".into(),
            exec: vec!["gnome-calculator".into()],
            icon: String::new(),
        }];
        for (c, t, w) in [
            ("", "", ""), // no focused window: no app name (no resident Finder)
            ("foot", "~", "Terminal"),
            ("org.gnome.Calculator", "x", "Calculator"),
            ("gnome-calculator", "", "Calculator"),
            ("", "xterm", "xterm"),
            ("weird", "T", "weird"),
        ] {
            assert_eq!(frontmost_name(c, t, PINS, &installed), w, "{c}");
        }
    }

    #[test]
    fn icons_by_class() {
        let installed = [Entry {
            id: "org.gnome.Calculator.desktop".into(),
            name: "Calculator".into(),
            exec: vec!["gnome-calculator".into()],
            icon: "accessories-calculator".into(),
        }];
        for (c, w) in [
            ("foot", "wolos-terminal"),
            ("Firefox", "wolos-browser"),
            ("gnome-calculator", "accessories-calculator"),
            ("weird", "weird"),
            ("Mail", "wolos-mail"),
        ] {
            assert_eq!(icon_for(c, PINS, &installed), w, "{c}");
        }
    }

    #[test]
    fn running_by_class() {
        let classes: std::collections::HashSet<String> =
            ["foot".to_string(), "firefox".into()].into();
        let running: Vec<&str> = PINS
            .iter()
            .filter(|p| is_running(p, &classes))
            .map(|p| p.name)
            .collect();
        assert_eq!(running, ["Browser", "Terminal"]);
    }
}
