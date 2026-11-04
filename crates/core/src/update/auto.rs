// SPDX-License-Identifier: BSD-2-Clause
//! Automatic updates (Settings > Software Update). The system half is
//! `wolos-update-download.timer`: it checks and downloads pending packages into pacman's cache
//! unless [`FILE`] says `download=off` (installing stays Software Update, with a restore point).
//! The app half is the per-account `wolos-app-update.timer` (`flatpak --user update`).

/// Written by wol-admind (`auto-update on|off`), read by the timer's script and Settings.
pub const FILE: &str = "/etc/wolos/auto-update";
/// User unit that installs Flathub app updates for this account.
pub const APPS_TIMER: &str = "wolos-app-update.timer";

/// The comment the file starts with.
const HEADER: &str = "# Written by WolOS (Settings > Software Update).\n";

/// File content for the download switch.
pub fn conf(download: bool) -> String {
    set(None, "download", download)
}

/// `current` with `key=on|off` set: the last line for `key` is replaced (none: one is added), every
/// other line stays, so changing `metered` keeps `download` and the other way round. No file yet:
/// the header and the one line. `key` is `download` or `metered`.
pub fn set(current: Option<&str>, key: &str, on: bool) -> String {
    let line = format!("{key}={}", if on { "on" } else { "off" });
    let Some(text) = current.filter(|t| !t.trim().is_empty()) else {
        return format!("{HEADER}{line}\n");
    };
    let is_key = |l: &str| {
        l.trim()
            .split_once('=')
            .is_some_and(|(k, _)| k.trim() == key)
    };
    let at = text
        .lines()
        .collect::<Vec<_>>()
        .iter()
        .rposition(|l| is_key(l));
    let mut lines: Vec<&str> = text.lines().collect();
    match at {
        Some(i) => lines[i] = &line,
        None => lines.push(&line),
    }
    format!("{}\n", lines.join("\n"))
}

/// Downloads also run on a metered connection only when the file says `metered=on`.
pub fn metered_allowed(text: Option<&str>) -> bool {
    text.is_some_and(|t| {
        t.lines()
            .filter_map(|l| l.trim().split_once('='))
            .rfind(|(k, _)| k.trim() == "metered")
            .is_some_and(|(_, v)| v.trim() == "on")
    })
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

    /// QA-100: the metered switch and the download switch share one file and keep each other's line.
    #[test]
    fn metered_switch_keeps_the_download_line() {
        assert!(!metered_allowed(None));
        assert!(!metered_allowed(Some(&conf(true))));
        let on = set(Some(&conf(false)), "metered", true);
        assert_eq!(
            on,
            "# Written by WolOS (Settings > Software Update).\ndownload=off\nmetered=on\n"
        );
        assert!(metered_allowed(Some(&on)) && !downloads(Some(&on)));
        let off = set(Some(&on), "metered", false);
        assert!(!metered_allowed(Some(&off)) && !downloads(Some(&off)));
        assert_eq!(off.matches("metered=").count(), 1, "{off}");
        // No file yet, or a hand-written one without a trailing newline.
        assert_eq!(set(None, "download", false), conf(false));
        assert_eq!(
            set(Some("metered=on"), "download", false),
            "metered=on\ndownload=off\n"
        );
        assert_eq!(
            set(Some("  metered = on\n"), "metered", false),
            "metered=off\n"
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
