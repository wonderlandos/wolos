//! Every Dock pin launches a program the image installs, and its window class names it.
//! Real-GPU run: the Dock pinned wol-files/wol-mail/wol-music/... ("No such file or directory").
//! WolOS apps are mature ports: each pin maps to a package in desktop/packages.txt (or a WolOS binary)
//! and its id is the app's Wayland app-id (= Hyprland class), so the running dot and menu bar name work.
use shell::apps::{PINS, TRASH, frontmost_name, icon_for, is_running};
use std::path::PathBuf;

/// Program -> (FreeBSD package, Wayland app-id). Verified in the 15.1 quarterly repo (pkg rquery, port
/// pkg-plist: bin/<program> + share/applications/<app-id>.desktop).
const PROVIDERS: &[(&str, &str, &str)] = &[
    ("nautilus", "nautilus", "org.gnome.Nautilus"),
    ("geary", "geary", "org.gnome.Geary"),
    ("amberol", "amberol", "io.bassi.Amberol"),
    ("shotwell", "shotwell", "org.gnome.Shotwell"),
    ("gnome-calendar", "gnome-calendar", "org.gnome.Calendar"),
    ("gnote", "gnote", "org.gnome.Gnote"),
    ("foot", "foot", "foot"),
    ("firefox", "firefox", "firefox"),
];

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

fn packages() -> Vec<String> {
    ["image/packages-base.txt", "desktop/packages.txt"]
        .iter()
        .flat_map(|f| {
            std::fs::read_to_string(repo(f))
                .unwrap_or_else(|e| panic!("{f}: {e}"))
                .lines()
                .filter_map(|l| l.split('#').next())
                .flat_map(|l| l.split_whitespace().map(String::from).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// WolOS binaries: installed by image/build-rust.sh or shipped in desktop/overlay/usr/local/bin.
fn wolos_binary(program: &str) -> bool {
    let build = std::fs::read_to_string(repo("image/build-rust.sh")).unwrap();
    let listed = build
        .lines()
        .find(|l| l.trim_start().starts_with("for b in wold "))
        .is_some_and(|l| {
            l.split_whitespace()
                .any(|w| w.trim_end_matches(';') == program)
        });
    listed
        || repo("desktop/overlay/usr/local/bin")
            .join(program)
            .is_file()
}

fn provided(program: &str, pkgs: &[String]) -> Result<(), String> {
    if wolos_binary(program) {
        return Ok(());
    }
    match PROVIDERS.iter().find(|(p, _, _)| *p == program) {
        Some((_, pkg, _)) if pkgs.iter().any(|x| x == pkg) => Ok(()),
        Some((_, pkg, _)) => Err(format!("{program}: package {pkg} not in packages.txt")),
        None => Err(format!("{program}: no WolOS binary and no package mapping")),
    }
}

#[test]
fn every_pin_and_trash_runs_an_installed_program() {
    let pkgs = packages();
    let mut bad: Vec<String> = PINS
        .iter()
        .map(|p| p.exec[0])
        .chain([TRASH[0]])
        .filter_map(|e| provided(e, &pkgs).err())
        .collect();
    bad.dedup();
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn packaged_pins_are_named_by_their_window_class() {
    for (program, _, app_id) in PROVIDERS {
        let Some(pin) = PINS.iter().find(|p| p.exec[0] == *program) else {
            continue;
        };
        assert_eq!(pin.id, *app_id, "{} pin id must be the app-id", pin.name);
        // Hyprland reports the app-id as the class; the Dock compares lowercased classes.
        assert_eq!(frontmost_name(app_id, "x", PINS, &[]), pin.name, "{app_id}");
        assert_eq!(icon_for(app_id, PINS, &[]), pin.icon, "{app_id}");
        assert!(
            is_running(pin, &[app_id.to_lowercase()].into()),
            "{app_id} not running"
        );
    }
}

#[test]
fn browser_pin_is_firefox() {
    // Firefox is the WolOS browser: its Wayland app-id (Hyprland class) is `firefox`.
    let b = PINS.iter().find(|p| p.name == "Browser").unwrap();
    assert_eq!((b.id, b.exec), ("firefox", &["firefox"][..]));
}

#[test]
fn pins_keep_wolos_names_and_icons() {
    let names: Vec<&str> = PINS.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        [
            "Files",
            "Launchpad",
            "Browser",
            "Mail",
            "Music",
            "Photos",
            "Calendar",
            "Notes",
            "Terminal",
            "App Store",
            "System Settings"
        ]
    );
    assert!(PINS.iter().all(|p| p.icon.starts_with("wolos-")));
}

#[test]
fn app_store_is_pinned_installed_and_opens_wol_packages() {
    // macOS Dock: App Store sits before System Settings.
    let p = PINS.iter().find(|p| p.name == "App Store").unwrap();
    assert_eq!(
        (p.id, p.icon, p.exec),
        ("org.wolos.store", "wolos-software", &["wol-store"][..])
    );
    assert!(wolos_binary("wol-store"), "wol-store not installed");
    let desktop = std::fs::read_to_string(repo(
        "desktop/overlay/usr/local/share/applications/org.wolos.store.desktop",
    ))
    .unwrap();
    for l in [
        "Name=App Store",
        "Exec=wol-store %f",
        "Icon=wolos-software",
        "MimeType=application/x-wolos-package;",
    ] {
        assert!(desktop.lines().any(|x| x == l), "{l}");
    }
    assert_eq!(
        frontmost_name("org.wolos.store", "", PINS, &[]),
        "App Store"
    );
}
