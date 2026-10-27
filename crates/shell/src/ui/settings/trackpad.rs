//! Trackpad & Mouse: pointer speed, scrolling and clicking (desktop prefs, applied live, journaled).
use super::*;

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("Trackpad & Mouse");
    let status = status_label();
    let d = e.desk.get();
    let toggle = |title: &str,
                  sub: &str,
                  on: bool,
                  set: fn(&mut wol_core::hyprprefs::DesktopPrefs, bool)| {
        let (dk, st) = (e.desk.clone(), status.clone());
        form::toggle_row(title, sub, on, move |v| dk.set(&st, |p| set(p, v)))
    };
    let (dk, st) = (e.desk.clone(), status.clone());
    p.append(&w::section(
        "",
        &[form::slider_row(
            "Tracking speed",
            (-1.0, 1.0, 0.1),
            f64::from(d.pointer_speed),
            ("Slow", "Fast"),
            move |v| dk.set(&st, |p| p.pointer_speed = v as f32),
        )],
    ));
    p.append(&w::section(
        "Trackpad",
        &[
            toggle(
                "Tap to click",
                "Tap with one finger.",
                d.tap_to_click,
                |p, v| p.tap_to_click = v,
            ),
            toggle(
                "Secondary click",
                "Click or tap with two fingers.",
                d.two_finger_secondary,
                |p, v| p.two_finger_secondary = v,
            ),
            toggle(
                "Natural scrolling",
                "Content follows your fingers.",
                d.touchpad_natural_scroll,
                |p, v| p.touchpad_natural_scroll = v,
            ),
            toggle(
                "Ignore the trackpad while typing",
                "",
                d.disable_while_typing,
                |p, v| p.disable_while_typing = v,
            ),
        ],
    ));
    p.append(&w::section(
        "Mouse",
        &[toggle(
            "Natural scrolling",
            "Content follows the wheel like a trackpad.",
            d.mouse_natural_scroll,
            |p, v| p.mouse_natural_scroll = v,
        )],
    ));
    p.append(&status);
    p
}
