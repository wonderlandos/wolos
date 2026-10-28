//! Automatic updates (Settings > Software Update). The system half is
//! `wolos-update-download.timer`: it checks and downloads pending packages into pacman's cache
//! unless [`FILE`] says `download=off` (installing stays Software Update, with a restore point).
//! The app half is the per-account `wolos-app-update.timer` (`flatpak --user update`).

/// Written by wol-admind (`auto-update on|off`), read by the timer's script and Settings.
pub const FILE: &str = "/etc/wolos/auto-update";
/// User unit that installs Flathub app updates for this account.
pub const APPS_TIMER: &str = "wolos-app-update.timer";

/// File content for the download switch.
pub fn conf(download: bool) -> String {
    format!(
        "# Written by WolOS (Settings > Software Update).\ndownload={}\n",
        if download { "on" } else { "off" }
    )
}

/// Downloads are on unless the file says `download=off` (missing file: the default, on).
pub fn downloads(text: Option<&str>) -> bool {
    !text.is_some_and(|t| {
        t.lines()
            .filter_map(|l| l.trim().split_once('='))
            .rfind(|(k, _)| k.trim() == "download")
            .is_some_and(|(_, v)| v.trim() == "off")
    })
}

/// `systemctl --user enable|disable --now wolos-app-update.timer`.
pub fn apps_argv(on: bool) -> [&'static str; 5] {
    [
        "systemctl",
        "--user",
        if on { "enable" } else { "disable" },
        "--now",
        APPS_TIMER,
    ]
}

/// `systemctl --user is-enabled` stdout -> whether app updates install themselves.
pub fn apps_enabled(out: Option<&str>) -> bool {
    out.is_some_and(|o| o.trim() == "enabled")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_switch_roundtrips_and_defaults_on() {
        assert!(downloads(None));
        assert!(downloads(Some("")));
        assert!(downloads(Some(&conf(true))));
        assert!(!downloads(Some(&conf(false))));
        assert!(downloads(Some("download=off\ndownload=on\n")), "last wins");
        assert_eq!(
            conf(false),
            "# Written by WolOS (Settings > Software Update).\ndownload=off\n"
        );
    }

    #[test]
    fn app_updates_follow_the_user_timer() {
        assert_eq!(
            apps_argv(true).join(" "),
            "systemctl --user enable --now wolos-app-update.timer"
        );
        assert_eq!(apps_argv(false)[2], "disable");
        assert!(apps_enabled(Some("enabled\n")));
        assert!(!apps_enabled(Some("disabled\n")));
        assert!(!apps_enabled(None));
    }
}
