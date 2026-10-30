//! Helium (default browser, Chromium) follows the WolOS theme through Chromium's GTK mode: the
//! window colours come from the GTK 3 theme `WolOS-<id>` (reloaded live, see `apps`), light/dark
//! and the accent preset from the settings portal, window buttons from `gtk-decoration-layout`.
//! Both files here are system-wide and theme-independent, so they ship in the image; nothing is
//! generated per user (a managed policy such as `BrowserThemeColor` would replace GTK mode with
//! a single colour and could not follow a per-user theme).
//! Verified against Chromium 156.0.8078.77: pref names in chrome/common/pref_names.h,
//! `ui::SystemTheme::kGtk = 1` (ui/color/system_theme.h), `BrowserColorScheme::kSystem = 0`
//! (chrome/browser/themes/theme_service.h), policy names in components/policy/resources.

use serde_json::json;

/// Package contract (the Helium port): one place for every path.
pub struct BrowserPaths {
    pub bin: &'static str,
    pub policies_dir: &'static str,
    /// Read next to the real executable on first run of a new profile: the package links
    /// `/opt/helium/initial_preferences` here.
    pub initial_prefs: &'static str,
}

pub const HELIUM: BrowserPaths = BrowserPaths {
    bin: "/usr/bin/helium",
    // Helium reads Chromium's managed-policy directory (strings in the 0.20.2.1 binary).
    policies_dir: "/etc/chromium/policies/managed",
    initial_prefs: "/usr/share/helium/initial_preferences",
};
/// Our file in `HELIUM.policies_dir`.
pub const POLICY_FILE: &str = "wolos.json";

/// New profiles start in GTK mode, follow the system light/dark, draw their own (GTK-styled)
/// title bar so the window buttons follow `gtk-decoration-layout`.
pub fn initial_preferences() -> String {
    let v = json!({
        "extensions": { "theme": { "system_theme": 1 } },
        "browser": {
            "theme": { "color_scheme2": 0 },
            "custom_chrome_frame": true,
            "show_home_button": false
        }
    });
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
}

/// Privacy defaults: no usage reports, no sign-in or sync, no promotional tabs.
pub fn policies() -> String {
    let v = json!({
        "MetricsReportingEnabled": false,
        "BrowserSignin": 0,
        "SyncDisabled": true,
        "PromotionalTabsEnabled": false,
        "UrlKeyedAnonymizedDataCollectionEnabled": false,
        "FeedbackSurveysEnabled": false,
        "BackgroundModeEnabled": false,
        "DefaultBrowserSettingEnabled": false
    });
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn overlay(abs: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../desktop/overlay")
            .join(abs.trim_start_matches('/'))
    }

    #[test]
    fn gtk_mode_and_system_colour_scheme_for_new_profiles() {
        let v: serde_json::Value = serde_json::from_str(&initial_preferences()).unwrap();
        assert_eq!(
            v["extensions"]["theme"]["system_theme"], 1,
            "ui::SystemTheme::kGtk"
        );
        assert_eq!(
            v["browser"]["theme"]["color_scheme2"], 0,
            "BrowserColorScheme::kSystem"
        );
        assert_eq!(v["browser"]["custom_chrome_frame"], true);
        let p: serde_json::Value = serde_json::from_str(&policies()).unwrap();
        assert_eq!(p["MetricsReportingEnabled"], false);
        assert_eq!(p["SyncDisabled"], true);
        // No telemetry, surveys or default-browser nagging (WolOS ships Helium as the default).
        for k in [
            "UrlKeyedAnonymizedDataCollectionEnabled",
            "FeedbackSurveysEnabled",
            "BackgroundModeEnabled",
            "DefaultBrowserSettingEnabled",
        ] {
            assert_eq!(p[k], false, "{k}");
        }
        assert!(
            p.get("BrowserThemeColor").is_none(),
            "would override GTK mode"
        );
    }

    /// Light/dark and the accent reach Helium (and Firefox, libadwaita) through the settings
    /// portal; xdg-desktop-portal-gtk serves it from the gsettings `wol theme` writes.
    #[test]
    fn settings_portal_is_the_gtk_backend() {
        let conf =
            std::fs::read_to_string(overlay("/etc/xdg/xdg-desktop-portal/hyprland-portals.conf"))
                .unwrap();
        assert!(
            conf.lines()
                .any(|l| l == "org.freedesktop.impl.portal.Settings=gtk")
        );
    }

    /// The wrapper names the desktop file that sets the Wayland app-id the Dock matches.
    #[test]
    fn package_contract_matches_the_recipe() {
        let pkg = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../image/repo/helium-browser/PKGBUILD"),
        )
        .unwrap();
        assert!(pkg.contains(HELIUM.bin));
        assert!(pkg.contains("/opt/helium/initial_preferences"));
        assert!(HELIUM.policies_dir.starts_with("/etc/chromium/"));
    }

    /// The image ships both files at the package paths; `WOL_REGEN=1` rewrites them.
    #[test]
    fn image_ships_them_at_the_package_paths() {
        let policy = format!("{}/{POLICY_FILE}", HELIUM.policies_dir);
        for (abs, text) in [
            (HELIUM.initial_prefs, initial_preferences()),
            (policy.as_str(), policies()),
        ] {
            let p = overlay(abs);
            if std::env::var_os("WOL_REGEN").is_some() {
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, &text).unwrap();
            }
            assert_eq!(std::fs::read_to_string(&p).ok(), Some(text), "{abs}");
        }
    }
}
