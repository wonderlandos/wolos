// SPDX-License-Identifier: BSD-2-Clause
//! The native backend's appearance inputs (`window::native::theme`, WK-W4): the theme id, the
//! contrast flag and the desktop preferences read from a config home, the Auto day/night switch,
//! the inotify watch seeing a change, and the file names matching what the shell writes.

use wolkit::window::native::theme::{self, Watch};

fn home(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wolkit-theme-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn no_files_follow_the_time_of_day() {
    let h = home("auto");
    let noon = theme::read(&h, (12, 0, 0));
    let night = theme::read(&h, (22, 30, 0));
    assert!(
        !noon.env.dark && night.env.dark,
        "Auto is light by day and dark at night"
    );
    assert!(
        noon.auto_switch_in.is_some_and(|s| s == 7 * 3600),
        "next switch at 19:00"
    );
    assert!(!noon.env.contrast && !noon.env.reduce_motion);
    assert!(
        noon.tokens.iter().filter(|t| **t != 0).count() >= 10,
        "the theme's tokens resolve"
    );
}

#[test]
fn files_set_theme_contrast_and_motion() {
    let h = home("files");
    std::fs::create_dir_all(h.join("wolos")).unwrap();
    std::fs::create_dir_all(h.join("wol")).unwrap();
    let dark = shared::theme::Variant::Dark.default_theme();
    std::fs::write(h.join(theme::THEME_FILE), format!("{dark}\n")).unwrap();
    std::fs::write(h.join(theme::CONTRAST_FILE), "").unwrap();
    std::fs::write(
        h.join(theme::PREFS_FILE),
        r#"{"reduce_motion": true, "reduce_transparency": true}"#,
    )
    .unwrap();
    let a = theme::read(&h, (12, 0, 0));
    assert!(a.env.dark, "a dark theme is dark at noon");
    assert!(a.auto_switch_in.is_none());
    assert!(a.env.contrast && a.env.reduce_motion && a.env.reduce_transparency);
}

#[test]
fn the_watch_sees_a_contrast_change() {
    let h = home("watch");
    std::fs::create_dir_all(h.join("wol")).unwrap();
    let mut w = Watch::new(&h).unwrap();
    assert!(w.poll().is_none(), "nothing changed yet");
    std::fs::write(h.join(theme::CONTRAST_FILE), "").unwrap();
    let mut ready = Vec::new();
    wolkit::loop_::wait(&w.fds(), 2000, &mut ready);
    let next = w.poll().expect("the change is seen");
    assert!(next.env.contrast);
}

#[test]
fn file_names_match_the_shell() {
    let style = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../shell/src/kit/style.rs"
    ))
    .unwrap();
    assert!(
        style.contains(&format!(
            "pub const CONTRAST_FILE: &str = \"{}\";",
            theme::CONTRAST_FILE
        )),
        "the shell's contrast flag file moved; follow it in window/native/theme.rs"
    );
    assert_eq!(theme::THEME_FILE, "wolos/theme");
    assert_eq!(theme::PREFS_FILE, "wolos/desktop.json");
}
