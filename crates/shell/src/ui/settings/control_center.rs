//! Control Center: which optional modules show (shell.json, applied with `reload`). Modules whose
//! hardware this computer lacks are not offered.
use super::*;

/// Always in Control Center: (icon, name).
const ALWAYS: [(&str, &str); 6] = [
    ("network-wireless-symbolic", "Wi-Fi"),
    ("bluetooth-active-symbolic", "Bluetooth"),
    ("weather-clear-night-symbolic", "Focus"),
    ("preferences-desktop-appearance-symbolic", "Dark Mode"),
    ("display-brightness-symbolic", "Display"),
    ("audio-volume-high-symbolic", "Sound"),
];

fn icon(key: &str) -> &'static str {
    match key {
        "now_playing" => "media-playback-start-symbolic",
        "battery" => "battery-full-symbolic",
        "keyboard_brightness" => "keyboard-brightness-symbolic",
        "accessibility" => "preferences-desktop-accessibility-symbolic",
        "night_shift" => "night-light-symbolic",
        "screenshot" => "camera-photo-symbolic",
        "record" => "media-record-symbolic",
        _ => "",
    }
}

/// Optional modules this computer can show (battery and keyboard backlight need the hardware).
pub(crate) fn offered(battery: bool, backlight: bool) -> Vec<(&'static str, &'static str)> {
    wol_core::prefs::ControlCenterPrefs::MODULES
        .into_iter()
        .filter(|(k, _)| match *k {
            "battery" => battery,
            "keyboard_brightness" => backlight,
            _ => true,
        })
        .collect()
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("Control Center");
    let always: Vec<gtk::Widget> = ALWAYS
        .iter()
        .map(|(i, n)| w::list_row(i, n, "", None).upcast())
        .collect();
    p.append(&w::section("Always Shown", &always));
    let cur = Rc::new(Cell::new(load_shell()));
    let sys = e.sys();
    let rows: Vec<gtk::Widget> =
        offered(sys.battery().is_some(), sys.keyboard_brightness().is_some())
            .into_iter()
            .map(|(key, label)| {
                let c = cur.clone();
                let on = cur.get().control_center.get(key).unwrap_or(false);
                let t = w::toggle(on, move |on| {
                    let mut v = c.get();
                    v.control_center.set(key, on);
                    c.set(v);
                    save_shell(v);
                });
                w::list_row(icon(key), label, "", Some(t.upcast_ref())).upcast()
            })
            .collect();
    p.append(&w::section("Show When You Want Them", &rows));
    p.append(&w::footnote(
        "Open Control Center from the menu bar. Changes show the next time it opens.",
    ));
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_modules_need_the_hardware() {
        let all = offered(true, true);
        assert_eq!(
            all.len(),
            wol_core::prefs::ControlCenterPrefs::MODULES.len()
        );
        let desktop = offered(false, false);
        assert!(
            !desktop
                .iter()
                .any(|m| m.0 == "battery" || m.0 == "keyboard_brightness")
        );
        assert!(
            all.iter().all(|m| !icon(m.0).is_empty()),
            "every module has an icon"
        );
    }
}
