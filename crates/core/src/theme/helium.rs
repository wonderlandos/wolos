// SPDX-License-Identifier: BSD-2-Clause
//! Helium (default browser, Chromium) follows the WolOS theme through Chromium's GTK mode: the
//! window colours come from the GTK 3 theme `WolOS-<id>` (reloaded live, see `apps`), light/dark
//! and the accent preset from the settings portal, window buttons from the compositor.
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

/// New profiles start in GTK mode and follow the system light/dark. They use the system title
/// bar: the compositor's strip, so Helium has the same window buttons as every other window.
pub fn initial_preferences() -> String {
    let v = json!({
        "extensions": { "theme": { "system_theme": 1 } },
        "browser": {
            "theme": { "color_scheme2": 0 },
            "custom_chrome_frame": false,
            "show_home_button": false
        }
    });
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
}

/// The browser's rows of the system TLS policy (ADR 0055), as managed policies so a page or an
/// extension cannot turn them off. Chromium 156 always offers the hybrid post-quantum group
/// X25519MLKEM768 first (its `PostQuantumKeyAgreementEnabled` policy ended at 146: there is no off
/// switch left to pin) and has TLS 1.2 as its floor (`SSLVersionMin` ended at 97), so neither needs
/// a row. Encrypted Client Hello hides the site's name from the network; it needs the site's HTTPS
/// DNS record, which only the built-in DNS client asks for (it sends its queries to the system
/// resolver, so DNS over TLS still applies). DNS over HTTPS is off: Chromium's "automatic" mode
/// would move lookups to a public DNS-over-HTTPS provider whenever the system's server is one it
/// knows, around the system resolver and its DNS-over-TLS choice (Settings > Network); pinned off,
/// every lookup goes to the system resolver. http:// pages are upgraded to https:// first.
/// Certificate transparency is enforced by default: no `CertificateTransparencyEnforcementDisabled*`
/// row is ever written (test below).
pub fn tls_policies() -> serde_json::Map<String, serde_json::Value> {
    let v = json!({
        "EncryptedClientHelloEnabled": true,
        "BuiltInDnsClientEnabled": true,
        "AdditionalDnsQueryTypesEnabled": true,
        "HttpsUpgradesEnabled": true,
        "DnsOverHttpsMode": "off"
    });
    match v {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

/// Privacy defaults: no usage reports, no sign-in or sync, no promotional tabs; then the browser
/// rows its web policy (`webpolicy/helium.json`) allows, and only those.
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
    // The TLS policy's browser rows (ADR 0055).
    let mut v = v;
    if let Some(m) = v.as_object_mut() {
        m.extend(tls_policies());
    }
    // The browser's web policy decides its runtime rows (tab discard, throttling; ADR 0042). A
    // refused policy adds none.
    if let (Some(m), Ok(p)) = (
        v.as_object_mut(),
        crate::webpolicy::parse(crate::webpolicy::HELIUM_JSON.as_bytes()),
    ) {
        m.extend(crate::webpolicy::browser_policies(&p));
    }
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
        assert_eq!(v["browser"]["custom_chrome_frame"], false);
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

    /// ADR 0055: ECH and the DNS records it needs are on, http is upgraded, and nothing weakens
    /// certificate checks or TLS (no CT exemption, no TLS-error override, no obsolete TLS rows).
    #[test]
    fn tls_rows_are_on_and_nothing_weakens_tls() {
        let p: serde_json::Value = serde_json::from_str(&policies()).unwrap();
        for k in [
            "EncryptedClientHelloEnabled",
            "BuiltInDnsClientEnabled",
            "AdditionalDnsQueryTypesEnabled",
            "HttpsUpgradesEnabled",
        ] {
            assert_eq!(p[k], true, "{k}");
        }
        assert_eq!(
            p["DnsOverHttpsMode"], "off",
            "lookups go to the system resolver (its DNS over TLS), never a public DoH provider"
        );
        assert!(p.get("DnsOverHttpsTemplates").is_none());
        let m = p.as_object().unwrap();
        for k in m.keys() {
            assert!(
                !k.starts_with("CertificateTransparencyEnforcementDisabled")
                    && !k.starts_with("SSLVersion")
                    && k != "PostQuantumKeyAgreementEnabled"
                    && k != "InsecureHashesInTLSHandshakesEnabled"
                    && k != "HSTSPolicyBypassList",
                "{k} weakens TLS or is not a policy of this engine"
            );
        }
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

    /// The paths the package must keep (it lives in wolos-pkgbuilds: its tools/lint.sh checks the
    /// recipe has `/usr/bin/helium` and `/opt/helium/initial_preferences`).
    #[test]
    fn package_contract_paths() {
        assert_eq!(HELIUM.bin, "/usr/bin/helium");
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
