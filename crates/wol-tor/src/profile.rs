// SPDX-License-Identifier: GPL-2.0-or-later
//! The Tor instance's profile at Tier 0 (ADR 0053 D5): managed policies and launch flags that
//! work on the repackaged browser with no engine patch.
//!
//! Defence in depth, in this order: the kernel (no route, [`crate::netns`]), the relay (CONNECT
//! only, [`crate::relay`]), then these. The proxy is set twice (policy and flag) and local name
//! resolution is mapped to "not found", so even a policy file that failed to bind leaves no name
//! resolved on this machine. Every policy key is checked against the tree's policy definitions by
//! the AT-8 validator before it ships [V].
use crate::units::RELAY_ADDR;
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

/// The directory of a profile that is bound over Atlas's policy directory.
pub const POLICY_SUBDIR: &str = "policies";
/// The managed-policy file inside [`POLICY_SUBDIR`] (Chromium reads `managed/*.json`).
pub const POLICY_FILE: &str = "managed/tor.json";
/// The font configuration file of the profile (`FONTCONFIG_FILE` of the browser unit).
pub const FONTS_FILE: &str = "fonts.conf";
/// Largest profile file read back by [`verify`].
const MAX_PROFILE_FILE: u64 = 1 << 20;

/// A fixed font set so the font list is the same for every Atlas Tor user: the bundled Noto and
/// Inter families only, no user fonts, cache on the unit's tmpfs [V: paths of the fonts packages].
pub const FONTS_CONF: &str = "<?xml version=\"1.0\"?>\n\
<!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n\
<fontconfig>\n\
  <dir>/usr/share/fonts/noto</dir>\n\
  <dir>/usr/share/fonts/inter</dir>\n\
  <cachedir>/tmp/fontconfig</cachedir>\n\
</fontconfig>\n";

/// The managed policies of the Tor instance (installed as `TOR_POLICY_DIR/managed/tor.json`).
pub fn policies() -> Value {
    json!({
        "ProxySettings": { "ProxyMode": "fixed_servers", "ProxyServer": format!("socks5://{RELAY_ADDR}") },
        "IncognitoModeAvailability": 2,
        "QuicAllowed": false,
        "WebRtcIPHandlingPolicy": "disable_non_proxied_udp",
        "HttpsOnlyMode": "force_enabled",
        "DnsOverHttpsMode": "off",
        "BuiltInDnsClientEnabled": false,
        "NetworkPredictionOptions": 2,
        "SearchSuggestEnabled": false,
        "DefaultJavaScriptJitSetting": 2,
        "DefaultWebUsbGuardSetting": 2,
        "DefaultWebHidGuardSetting": 2,
        "DefaultSerialGuardSetting": 2,
        "DefaultWebBluetoothGuardSetting": 2,
        "DefaultGeolocationSetting": 2,
        "DefaultSensorsSetting": 2,
        "DefaultNotificationsSetting": 2,
        "AudioCaptureAllowed": false,
        "VideoCaptureAllowed": false,
        "ScreenCaptureAllowed": false,
        "PasswordManagerEnabled": false,
        "AutofillAddressEnabled": false,
        "AutofillCreditCardEnabled": false,
        "SavingBrowserHistoryDisabled": true,
        "BrowserSignin": 0,
        "SyncDisabled": true,
        "MetricsReportingEnabled": false,
        "BackgroundModeEnabled": false,
        "BrowserNetworkTimeQueriesEnabled": false,
        "SpellCheckServiceEnabled": false,
        "TranslateEnabled": false,
        "ExtensionInstallBlocklist": ["*"],
        // The design palette's violet (icon.categories.wolos.body): the Tor window's colour
        // without a new token and without an engine patch (ADR 0053 D6).
        "BrowserThemeColor": "#6A4DF4"
    })
}

/// The flags Atlas starts with in the Tor instance; `profile_dir` is on the unit's private tmpfs.
pub fn atlas_flags(profile_dir: &Path) -> Vec<String> {
    let mut v = vec![
        format!("--user-data-dir={}", profile_dir.display()),
        "--incognito".to_string(),
        format!("--proxy-server=socks5://{RELAY_ADDR}"),
        "--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1".to_string(),
        "--lang=en-US".to_string(),
        "--accept-lang=en-US,en".to_string(),
        "--no-pings".to_string(),
        "--disable-crash-reporter".to_string(),
        "--disable-background-networking".to_string(),
        "--no-first-run".to_string(),
        "--class=atlas-tor".to_string(),
        // Read by overlay patches 0007 (SOCKS isolation credential) and 0008 (Tor window
        // chrome); the repackaged engine ignores switches it does not know.
        "--atlas-tor-window".to_string(),
        "--atlas-tor-isolation".to_string(),
    ];
    // [V] feature names at the engine version of the lane build (AT-1 `--enable-features` dump).
    v.push("--disable-features=WebGPU,WebTransport,DirectSockets,MediaRouter".to_string());
    v
}

fn write_private(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(text.as_bytes())
}

/// Writes the policy file and `fonts.conf` into `dir` (the launcher does this before the browser
/// unit starts; the unit binds `dir/policies` read-only over Atlas's policy directory).
pub fn write_to(dir: &Path) -> io::Result<()> {
    let json = serde_json::to_string_pretty(&policies()).map_err(io::Error::other)?;
    write_private(&dir.join(POLICY_SUBDIR).join(POLICY_FILE), &json)?;
    write_private(&dir.join(FONTS_FILE), FONTS_CONF)
}

fn read_small(path: &Path) -> Result<String, String> {
    let mut s = String::new();
    fs::File::open(path)
        .and_then(|f| f.take(MAX_PROFILE_FILE).read_to_string(&mut s))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(s)
}

/// Checks that `dir` holds the Tor policies and `fonts.conf` Atlas must start with: the policy
/// file parses, names the relay as the only proxy and forces the private profile. The host calls
/// this before it opens the relay, so a missing or altered profile means Atlas never starts.
pub fn verify(dir: &Path) -> Result<(), String> {
    let text = read_small(&dir.join(POLICY_SUBDIR).join(POLICY_FILE))?;
    let doc: Value = serde_json::from_str(&text)
        .map_err(|e| format!("the Tor policy file is not valid JSON: {e}"))?;
    let want = policies();
    for key in ["ProxySettings", "IncognitoModeAvailability", "QuicAllowed"] {
        if doc.get(key) != want.get(key) {
            return Err(format!(
                "the Tor policy file does not set {key} as required"
            ));
        }
    }
    read_small(&dir.join(FONTS_FILE)).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policies_force_the_relay_and_close_side_channels() {
        let p = policies();
        assert_eq!(p["ProxySettings"]["ProxyServer"], "socks5://127.0.0.1:9150");
        assert_eq!(p["IncognitoModeAvailability"], 2);
        assert_eq!(p["QuicAllowed"], false);
        assert_eq!(p["WebRtcIPHandlingPolicy"], "disable_non_proxied_udp");
        assert_eq!(p["HttpsOnlyMode"], "force_enabled");
        assert_eq!(p["DefaultJavaScriptJitSetting"], 2);
        assert_eq!(p["ExtensionInstallBlocklist"][0], "*");
        assert_eq!(p["BrowserThemeColor"], "#6A4DF4");
    }

    #[test]
    fn theme_colour_is_the_design_palette_violet() {
        // No new token (ADR 0053 D6): the value must stay the wolos category body colour.
        let tokens = include_str!("../../../design/tokens/colors.json");
        let doc: Value = serde_json::from_str(tokens).unwrap();
        let body = doc["icon"]["categories"]["wolos"]["body"]
            .as_str()
            .or_else(|| doc["categories"]["wolos"]["body"].as_str())
            .expect("wolos body colour in design/tokens/colors.json");
        assert_eq!(policies()["BrowserThemeColor"], body);
    }

    #[test]
    fn the_switches_are_the_ones_the_patches_read() {
        // Overlay patches 0007/0008/0009 test exactly these two names (ADR 0053 D6).
        let f = atlas_flags(Path::new("/p"));
        assert_eq!(f.iter().filter(|x| *x == "--atlas-tor-window").count(), 1);
        assert_eq!(
            f.iter().filter(|x| *x == "--atlas-tor-isolation").count(),
            1
        );
    }

    #[test]
    fn local_addresses_are_never_exposed_by_a_disabled_mdns_mask() {
        // WebRtcHideLocalIpsWithMdns hides local ICE addresses; disabling it would expose them.
        let f = atlas_flags(Path::new("/p")).join(" ");
        assert!(!f.contains("WebRtcHideLocalIpsWithMdns"), "{f}");
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wol-tor-profile-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_profile_is_written_with_the_policy_keys_and_fonts() {
        let d = scratch("write");
        write_to(&d).unwrap();
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(d.join("policies/managed/tor.json")).unwrap())
                .unwrap();
        assert_eq!(
            doc["ProxySettings"]["ProxyServer"],
            "socks5://127.0.0.1:9150"
        );
        assert_eq!(doc["BrowserThemeColor"], "#6A4DF4");
        assert!(
            fs::read_to_string(d.join("fonts.conf"))
                .unwrap()
                .contains("<fontconfig>")
        );
        assert_eq!(verify(&d), Ok(()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_or_altered_profile_fails_closed() {
        let d = scratch("closed");
        assert!(verify(&d).is_err(), "nothing written: Atlas must not start");
        write_to(&d).unwrap();
        fs::remove_file(d.join("fonts.conf")).unwrap();
        assert!(verify(&d).is_err(), "no fonts.conf");
        write_to(&d).unwrap();
        let mut doc = policies();
        doc["QuicAllowed"] = true.into();
        fs::write(d.join("policies/managed/tor.json"), doc.to_string()).unwrap();
        assert!(verify(&d).unwrap_err().contains("QuicAllowed"));
        let mut doc = policies();
        doc["ProxySettings"]["ProxyMode"] = "direct".into();
        fs::write(d.join("policies/managed/tor.json"), doc.to_string()).unwrap();
        assert!(verify(&d).unwrap_err().contains("ProxySettings"));
        fs::write(d.join("policies/managed/tor.json"), "{").unwrap();
        assert!(verify(&d).unwrap_err().contains("not valid JSON"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn flags_resolve_nothing_locally() {
        let f = atlas_flags(Path::new("/private/atlas-tor"));
        assert_eq!(f[0], "--user-data-dir=/private/atlas-tor");
        assert!(f.contains(&"--incognito".to_string()));
        assert!(
            f.contains(&"--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1".to_string())
        );
        assert!(
            f.iter()
                .any(|x| x == "--proxy-server=socks5://127.0.0.1:9150")
        );
        assert!(
            !f.iter().any(|x| x.contains("no-sandbox")),
            "the engine sandbox stays on (patches-0 L-07)"
        );
        assert!(
            f.contains(&"--atlas-tor-window".to_string())
                && f.contains(&"--atlas-tor-isolation".to_string())
        );
    }
}
