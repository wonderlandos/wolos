// SPDX-License-Identifier: BSD-2-Clause
//! Atlas, the WolOS browser: the package contract (every path in one place) and what WolOS hands
//! it as data: managed and recommended policies, the first-run preferences of a new profile and
//! the launcher's default switches.
//!
//! Theme: Atlas follows the WolOS theme through the engine's GTK mode. The window colours come
//! from the GTK 3 theme `WolOS-<id>` (reloaded live, see `theme::apps`), light/dark and the accent
//! preset from the settings portal, window buttons from the compositor. Every file here is
//! system-wide and theme-independent, so it ships in the image; nothing is generated per user (a
//! managed policy such as `BrowserThemeColor` would replace GTK mode with a single colour and
//! could not follow a per-user theme).
//!
//! Engine names, verified against 156.0.8078.77: pref names in chrome/common/pref_names.h,
//! `ui::SystemTheme::kGtk = 1` (ui/color/system_theme.h), `BrowserColorScheme::kSystem = 0`
//! (chrome/browser/themes/theme_service.h), policy names in components/policy/resources; at
//! 156.0.8078.79 (prepared tree): `vertical_tabs.uncollapsed_width` and
//! `vertical_tabs.expand_on_hover` (chrome/common/pref_names.h) and the policy types in
//! `tests/fixtures/atlas-policies.tsv`. The layout pref key is an internal identifier of the
//! engine overlay (classic 0, compact 1, vertical 2, dynamic 3) and keeps its engine spelling.
//!
//! Invariant: every policy key WolOS writes is a policy the shipped engine defines, with that
//! policy's value type (`every_policy_is_one_the_engine_defines`).

use serde_json::{Map, Value, json};

/// Package contract (wolos-pkgbuilds `atlas-browser`): one place for every path.
pub struct BrowserPaths {
    /// The package that installs the browser.
    pub package: &'static str,
    /// The launcher (`atlas.sh` in the recipe): WolOS defaults, then the flags files, then the
    /// command line.
    pub bin: &'static str,
    /// Where the package unpacks the engine.
    pub install_dir: &'static str,
    /// The desktop entry; its stem is the Wayland app id, the compositor's window class.
    pub desktop_id: &'static str,
    /// Environment variable the launcher sets to `desktop_id` (the engine names its windows
    /// after it; unset, it falls back to the same name).
    pub desktop_env: &'static str,
    /// Window class (Wayland app id) of every normal window: the Dock pin, menu bar and window
    /// rules match it.
    pub class: &'static str,
    /// Mandatory policies: the person cannot change these.
    pub policies_dir: &'static str,
    /// Recommended policies: defaults the person can change in Settings.
    pub recommended_dir: &'static str,
    /// Where the engine reads policies today (its built-in policy path, 156.0.8078.79): the same
    /// `managed/` and `recommended/` files are written there too. TEMPORARY: the commit that
    /// carries the fork's policy-path patch (T-04) removes this field and its two files.
    pub legacy_policies_dir: &'static str,
    /// Read next to the real executable on first run of a new profile: the package links
    /// `<install_dir>/initial_preferences` here.
    pub initial_prefs: &'static str,
    /// System flags file, read by the launcher after its defaults.
    pub flags_file: &'static str,
    /// The person's flags file, under `$XDG_CONFIG_HOME`, read after the system one.
    pub user_flags: &'static str,
    /// User data and cache directories, under `$XDG_CONFIG_HOME` and `$XDG_CACHE_HOME`.
    pub profile_dir: &'static str,
    pub cache_dir: &'static str,
}

pub const ATLAS: BrowserPaths = BrowserPaths {
    package: "atlas-browser",
    bin: "/usr/bin/atlas",
    install_dir: "/opt/atlas",
    desktop_id: "atlas.desktop",
    desktop_env: "ATLAS_DESKTOP",
    class: "atlas",
    policies_dir: "/etc/atlas/policies/managed",
    recommended_dir: "/etc/atlas/policies/recommended",
    legacy_policies_dir: "/etc/chromium/policies",
    initial_prefs: "/usr/share/atlas/initial_preferences",
    flags_file: "/etc/atlas-flags.conf",
    user_flags: "atlas-flags.conf",
    profile_dir: "atlas",
    cache_dir: "atlas",
};
/// Our file in `ATLAS.policies_dir` and in `ATLAS.recommended_dir`.
pub const POLICY_FILE: &str = "wolos.json";

/// The names the browser WolOS shipped before Atlas left on a person's disk. The launcher moves
/// them once (wolos-pkgbuilds `atlas-browser/atlas.sh`, the profile migration):
/// 1. `~/.config/atlas` absent and `~/.config/<config>` present: rename it (same filesystem, one
///    atomic step), the same for `~/.cache/<cache>`;
/// 2. `~/.config/mimeapps.list`: entries naming `desktop_id` name `ATLAS.desktop_id` instead;
/// 3. `~/.config/<flags>`: read once with a note that `ATLAS.user_flags` is the file now, then
///    ignored;
/// 4. the keyring entry stays as it is (cookies and saved sessions keep working);
/// 5. a saved Dock layout pinning `pin` shows the browser pin (the shell maps it,
///    `shell::apps::current_id`).
pub struct Legacy {
    pub config: &'static str,
    pub cache: &'static str,
    pub desktop_id: &'static str,
    pub flags: &'static str,
    pub pin: &'static str,
}

pub const LEGACY: Legacy = Legacy {
    config: "net.imput.helium",
    cache: "net.imput.helium",
    desktop_id: "helium.desktop",
    flags: "helium-flags.conf",
    pin: "helium",
};

/// Width of the tab sidebar in a new profile, in px (the reel's sidebar, tokens.md).
pub const SIDEBAR_WIDTH: u32 = 220;
/// Vertical tabs: the tab strip is the sidebar.
pub const LAYOUT_VERTICAL: u32 = 2;

/// New profiles start in GTK mode, follow the system light/dark, use the compositor's title
/// strip (the same window buttons as every other window) and keep their tabs in a sidebar of
/// [`SIDEBAR_WIDTH`] that stays put under the pointer. The first-run page is not marked done:
/// it opens once (`--no-first-run` skips it).
pub fn initial_preferences() -> String {
    let v = json!({
        "extensions": { "theme": { "system_theme": 1 } },
        "browser": {
            "theme": { "color_scheme2": 0 },
            "custom_chrome_frame": false,
            "show_home_button": false
        },
        "helium": { "browser": { "layout": LAYOUT_VERTICAL } },
        "vertical_tabs": {
            "uncollapsed_width": SIDEBAR_WIDTH,
            "expand_on_hover": false
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
pub fn tls_policies() -> Map<String, Value> {
    let v = json!({
        "EncryptedClientHelloEnabled": true,
        "BuiltInDnsClientEnabled": true,
        "AdditionalDnsQueryTypesEnabled": true,
        "HttpsUpgradesEnabled": true,
        "DnsOverHttpsMode": "off"
    });
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// Mandatory policies: no usage reports, no sign-in or sync, no promotional tabs, no surveys, no
/// clock queries to a time server, no ad measurement or topics, no cast discovery, no spelling
/// sent to a service and no generative features. Then come the runtime rows that the browser's
/// web policy (`webpolicy/atlas.json`) allows, and only those.
pub fn policies() -> String {
    let mut v = json!({
        "MetricsReportingEnabled": false,
        "BrowserSignin": 0,
        "SyncDisabled": true,
        "PromotionalTabsEnabled": false,
        "UrlKeyedAnonymizedDataCollectionEnabled": false,
        "FeedbackSurveysEnabled": false,
        "BackgroundModeEnabled": false,
        "DefaultBrowserSettingEnabled": false,
        "BrowserNetworkTimeQueriesEnabled": false,
        "PrivacySandboxPromptEnabled": false,
        "PrivacySandboxAdTopicsEnabled": false,
        "PrivacySandboxSiteEnabledAdsEnabled": false,
        "PrivacySandboxAdMeasurementEnabled": false,
        "EnableMediaRouter": false,
        "SpellCheckServiceEnabled": false,
        "GenAiDefaultSettings": 2
    });
    // The TLS policy's browser rows (ADR 0055).
    if let Some(m) = v.as_object_mut() {
        m.extend(tls_policies());
    }
    // The browser's web policy decides its runtime rows (tab discard, throttling; ADR 0042). A
    // refused policy adds none.
    if let (Some(m), Ok(p)) = (
        v.as_object_mut(),
        crate::webpolicy::parse(crate::webpolicy::ATLAS_JSON.as_bytes()),
    ) {
        m.extend(crate::webpolicy::browser_policies(&p));
    }
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
}

/// Recommended policies: what is typed in the address bar stays on the computer (no
/// suggestions from the search provider, no preloading of pages the person did not open) and
/// pages are not sent to a translation service. Each is a switch in Settings the person can turn
/// on (decision D2 of the Atlas program).
pub fn recommended_policies() -> String {
    let v = json!({
        "SearchSuggestEnabled": false,
        "NetworkPredictionOptions": 2,
        "TranslateEnabled": false
    });
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
}

/// When the launcher adds a default switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Always,
    /// The session is a Wayland one (`WAYLAND_DISPLAY` set and not empty).
    Wayland,
    /// It is not: the engine picks the platform itself.
    NoWayland,
    /// The machine has a GPU render node (`/dev/dri/renderD*`): a VM without one stays in
    /// software.
    RenderNode,
}

/// One default switch of the launcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchFlag {
    pub flag: &'static str,
    pub when: When,
}

/// The launcher's defaults, in order. The flags files and the command line come after them, so
/// a person's own switch wins.
pub fn launcher_flags() -> &'static [LaunchFlag] {
    &[
        // Cookies and saved passwords are encrypted with a key kept in the login keyring (the
        // session's desktop name is not one the engine maps to it by itself).
        LaunchFlag {
            flag: "--password-store=gnome-libsecret",
            when: When::Always,
        },
        LaunchFlag {
            flag: "--ozone-platform=wayland",
            when: When::Wayland,
        },
        LaunchFlag {
            flag: "--ozone-platform-hint=auto",
            when: When::NoWayland,
        },
        // Hardware video decode and encode (VA-API).
        LaunchFlag {
            flag: "--enable-features=AcceleratedVideoDecodeLinuxGL,AcceleratedVideoEncoder",
            when: When::RenderNode,
        },
    ]
}

/// The defaults the launcher adds on a machine: `wayland` is a non-empty `WAYLAND_DISPLAY`,
/// `render_node` a `/dev/dri/renderD*` that exists.
pub fn launcher_defaults(wayland: bool, render_node: bool) -> Vec<&'static str> {
    launcher_flags()
        .iter()
        .filter(|f| match f.when {
            When::Always => true,
            When::Wayland => wayland,
            When::NoWayland => !wayland,
            When::RenderNode => render_node,
        })
        .map(|f| f.flag)
        .collect()
}

/// Every policy key and value WolOS ships: (`managed`, key, value).
pub fn shipped_policies() -> Vec<(bool, String, Value)> {
    let mut out = Vec::new();
    for (managed, text) in [(true, policies()), (false, recommended_policies())] {
        let m: Map<String, Value> = serde_json::from_str(&text).unwrap_or_default();
        out.extend(m.into_iter().map(|(k, v)| (managed, k, v)));
    }
    out
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

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn gtk_mode_and_system_colour_scheme_for_new_profiles() {
        let v = json(&initial_preferences());
        assert_eq!(
            v["extensions"]["theme"]["system_theme"], 1,
            "ui::SystemTheme::kGtk"
        );
        assert_eq!(
            v["browser"]["theme"]["color_scheme2"], 0,
            "BrowserColorScheme::kSystem"
        );
        assert_eq!(v["browser"]["custom_chrome_frame"], false);
        assert_eq!(v["browser"]["show_home_button"], false);
        let p = json(&policies());
        assert_eq!(p["MetricsReportingEnabled"], false);
        assert_eq!(p["SyncDisabled"], true);
        assert_eq!(p["BrowserSignin"], 0);
        // No telemetry, surveys or default-browser nagging (WolOS ships Atlas as the default).
        for k in [
            "PromotionalTabsEnabled",
            "UrlKeyedAnonymizedDataCollectionEnabled",
            "FeedbackSurveysEnabled",
            "BackgroundModeEnabled",
            "DefaultBrowserSettingEnabled",
        ] {
            assert_eq!(p[k], false, "{k}");
        }
        for text in [policies(), recommended_policies()] {
            assert!(
                json(&text).get("BrowserThemeColor").is_none(),
                "would override GTK mode"
            );
        }
    }

    /// The reel's window: tabs in a vertical sidebar 220 px wide that does not grow under the
    /// pointer; the first-run page still opens once.
    #[test]
    fn new_profiles_keep_their_tabs_in_a_sidebar_of_220() {
        let v = json(&initial_preferences());
        assert_eq!(v["helium"]["browser"]["layout"], 2, "vertical");
        assert_eq!(v["vertical_tabs"]["uncollapsed_width"], 220);
        assert_eq!(v["vertical_tabs"]["expand_on_hover"], false);
        let text = initial_preferences();
        assert!(
            !text.contains("completed_onboarding"),
            "first run opens once"
        );
        assert!(
            !text.contains("vertical_right_aligned"),
            "engine default: left"
        );
    }

    /// Nothing calls home: the privacy rows of the program's AT-8 and D2.
    #[test]
    fn privacy_policies_keep_what_you_do_on_this_computer() {
        let p = json(&policies());
        for k in [
            "BrowserNetworkTimeQueriesEnabled",
            "PrivacySandboxPromptEnabled",
            "PrivacySandboxAdTopicsEnabled",
            "PrivacySandboxSiteEnabledAdsEnabled",
            "PrivacySandboxAdMeasurementEnabled",
            "EnableMediaRouter",
            "SpellCheckServiceEnabled",
        ] {
            assert_eq!(p[k], false, "{k}");
        }
        assert_eq!(p["GenAiDefaultSettings"], 2, "not allowed");
        // Typing stays local, but the person can turn suggestions back on: recommended, not
        // mandatory, and in no other file.
        let r = json(&recommended_policies());
        assert_eq!(r["SearchSuggestEnabled"], false);
        assert_eq!(r["NetworkPredictionOptions"], 2, "never predict");
        assert_eq!(r["TranslateEnabled"], false);
        for k in r.as_object().unwrap().keys() {
            assert!(p.get(k).is_none(), "{k} in both files");
        }
    }

    /// ADR 0055: ECH and the DNS records it needs are on, http is upgraded, and nothing weakens
    /// certificate checks or TLS (no CT exemption, no TLS-error override, no obsolete TLS rows).
    #[test]
    fn tls_rows_are_on_and_nothing_weakens_tls() {
        let p = json(&policies());
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

    /// The policy list the engine build defines (AT-1 dumps it from the prepared tree; until
    /// then a stub fixture written from components/policy/resources): name, type, allowed ints,
    /// whether it can be recommended.
    fn engine_policies() -> Vec<(String, String, Vec<i64>, bool)> {
        include_str!("../tests/fixtures/atlas-policies.tsv")
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let mut w = l.split('\t');
                let name = w.next().unwrap_or_default().to_owned();
                let ty = w.next().unwrap_or_default().to_owned();
                let ints = w
                    .next()
                    .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
                    .unwrap_or_default();
                let recommendable = w.next() == Some("recommendable");
                (name, ty, ints, recommendable)
            })
            .collect()
    }

    /// A misspelt key is ignored by the engine without a word: every key we ship must be in the
    /// engine's list with the right type, and an enum value one the engine accepts.
    #[test]
    fn every_policy_is_one_the_engine_defines() {
        let defs = engine_policies();
        assert!(defs.len() >= 20, "fixture read");
        let shipped = shipped_policies();
        assert!(shipped.len() >= 19);
        for (managed, k, v) in shipped {
            let (_, ty, ints, recommendable) = defs
                .iter()
                .find(|d| d.0 == k)
                .unwrap_or_else(|| panic!("{k}: not a policy of the engine"));
            match ty.as_str() {
                "bool" => assert!(v.is_boolean(), "{k}: {v} is not a boolean"),
                "int" => {
                    let n = v.as_i64().unwrap_or_else(|| panic!("{k}: {v} not an int"));
                    assert!(
                        ints.is_empty() || ints.contains(&n),
                        "{k}: {n} not in {ints:?}"
                    );
                }
                t => panic!("{k}: unknown type {t} in the fixture"),
            }
            assert!(managed || *recommendable, "{k}: cannot be recommended");
        }
    }

    #[test]
    fn launcher_defaults_follow_the_machine() {
        let wayland = "--ozone-platform=wayland";
        let auto = "--ozone-platform-hint=auto";
        let vaapi = "--enable-features=AcceleratedVideoDecodeLinuxGL,AcceleratedVideoEncoder";
        let keyring = "--password-store=gnome-libsecret";
        assert_eq!(launcher_defaults(true, true), [keyring, wayland, vaapi]);
        assert_eq!(launcher_defaults(false, true), [keyring, auto, vaapi]);
        assert_eq!(launcher_defaults(true, false), [keyring, wayland]);
        assert_eq!(launcher_defaults(false, false), [keyring, auto]);
        for f in launcher_flags() {
            assert!(f.flag.starts_with("--") && !f.flag.contains(' '), "{f:?}");
        }
    }

    /// Light/dark and the accent reach Atlas (and libadwaita apps) through the settings portal;
    /// xdg-desktop-portal-gtk serves it from the gsettings `wol theme` writes.
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

    /// The paths the package must keep (wolos-pkgbuilds `atlas-browser`: its tools/lint.sh checks
    /// the recipe installs `/usr/bin/atlas` and links `/opt/atlas/initial_preferences`).
    #[test]
    fn package_contract_paths() {
        assert_eq!(ATLAS.package, "atlas-browser");
        assert_eq!(ATLAS.bin, "/usr/bin/atlas");
        assert_eq!(ATLAS.install_dir, "/opt/atlas");
        assert_eq!(
            ATLAS.desktop_id.strip_suffix(".desktop"),
            Some(ATLAS.class),
            "the app id is the desktop entry's stem"
        );
        assert_eq!(ATLAS.bin.rsplit('/').next(), Some(ATLAS.class));
        assert!(ATLAS.policies_dir.starts_with("/etc/atlas/policies/"));
        assert_eq!(
            Path::new(ATLAS.policies_dir).parent(),
            Path::new(ATLAS.recommended_dir).parent()
        );
        assert_eq!(ATLAS.initial_prefs, "/usr/share/atlas/initial_preferences");
        assert_eq!(ATLAS.flags_file, "/etc/atlas-flags.conf");
        assert_eq!(
            Some(ATLAS.user_flags),
            Path::new(ATLAS.flags_file)
                .file_name()
                .and_then(|n| n.to_str())
        );
        assert_ne!(LEGACY.config, ATLAS.profile_dir);
        assert_ne!(LEGACY.desktop_id, ATLAS.desktop_id);
    }

    /// The image ships the three files at the package paths; `WOL_REGEN=1` rewrites them. Until
    /// the policy-path patch (T-04) lands, the engine's built-in policy path carries the same
    /// two policy files; the old browser's first-run file is gone.
    #[test]
    fn image_ships_them_at_the_package_paths() {
        let managed = format!("{}/{POLICY_FILE}", ATLAS.policies_dir);
        let recommended = format!("{}/{POLICY_FILE}", ATLAS.recommended_dir);
        let old_managed = format!("{}/managed/{POLICY_FILE}", ATLAS.legacy_policies_dir);
        let old_recommended = format!("{}/recommended/{POLICY_FILE}", ATLAS.legacy_policies_dir);
        for (abs, text) in [
            (ATLAS.initial_prefs, initial_preferences()),
            (managed.as_str(), policies()),
            (recommended.as_str(), recommended_policies()),
            (old_managed.as_str(), policies()),
            (old_recommended.as_str(), recommended_policies()),
        ] {
            let p = overlay(abs);
            if std::env::var_os("WOL_REGEN").is_some() {
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, &text).unwrap();
            }
            assert_eq!(std::fs::read_to_string(&p).ok(), Some(text), "{abs}");
        }
        // The old path is the new path, byte for byte, while both ship.
        for (old, new) in [(&old_managed, &managed), (&old_recommended, &recommended)] {
            assert_eq!(
                std::fs::read(overlay(old)).ok(),
                std::fs::read(overlay(new)).ok(),
                "{old}"
            );
        }
        assert!(!overlay("/usr/share/helium").exists());
    }

    /// The shipped files name the old browser only in pref keys the engine keeps.
    #[test]
    fn shipped_files_name_no_old_brand() {
        for text in [initial_preferences(), policies(), recommended_policies()] {
            let rest = text.replace("\"helium\": {\n    \"browser\"", "");
            assert!(!rest.to_lowercase().contains("helium"), "{text}");
        }
    }
}
