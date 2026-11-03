// SPDX-License-Identifier: BSD-2-Clause
//! `webengine::Policy`, frozen (plan-wave3 6.4): the per-app policy layer's output for track 2. WA-4
//! reads `/usr/share/wolos/webpolicy/<app>.json` (which also carries track 1's fields) and hands
//! this the five fields it has: the origins the app may reach, the script heap cap, what telemetry
//! is refused, whether a hidden window is throttled, and the web features the app needs. As JSON
//! (`Policy::from_json`), an unknown field is refused and a missing one is its default: no origin,
//! no cap, nothing refused beyond the origin list, throttled, no optional feature.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    /// Origins the app may reach (`https://example.com`, `https://example.com:8443`), or
    /// `https://*.example.com` for every subdomain (not the bare domain), or `file:///dir/` for a
    /// folder of the app's own pages. Anything else is refused ([`crate::sandbox`]).
    pub origins: Vec<String>,
    /// Script heap cap in MiB; `None` is the engine's default.
    pub heap_mb: Option<u32>,
    /// Hosts and paths refused even inside an allowed origin (beacons, analytics).
    pub telemetry_off: TelemetryOff,
    /// A hidden window's timers, animations and painting are throttled by the engine.
    pub background_throttle: bool,
    /// Web features the app needs (WA-6 vocabulary); see [`FEATURES`].
    pub features: Vec<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            origins: Vec::new(),
            heap_mb: None,
            telemetry_off: TelemetryOff::default(),
            background_throttle: true,
            features: Vec::new(),
        }
    }
}

/// The same JSON object as `telemetry_off` in WA-4's policy files (`hosts`, `beacon_paths`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TelemetryOff {
    /// Hosts never reached (`stats.example.com`; a leading `*.` for every subdomain).
    pub hosts: Vec<String>,
    /// Path prefixes refused on every host (`/beacon`, `/api/telemetry`).
    pub beacon_paths: Vec<String>,
}

/// Feature ids the engine switches, with their `wwe_config.features` bit
/// (`engine/include/wolwebengine.h`). A feature not listed here is not switchable at the pin: it is
/// on or off in the engine whatever the policy says, and WA-6's probes report whether it works.
pub const FEATURES: &[(&str, u32)] = &[
    ("webgl2", crate::ffi::FEATURE_WEBGL2),
    ("webgpu", crate::ffi::FEATURE_WEBGPU),
    ("service-workers", crate::ffi::FEATURE_SERVICE_WORKER),
    ("indexeddb", crate::ffi::FEATURE_INDEXEDDB),
    ("notifications", crate::ffi::FEATURE_NOTIFICATIONS),
    ("webrtc", crate::ffi::FEATURE_WEBRTC),
];

impl Policy {
    /// Reads a policy file's JSON.
    pub fn from_json(text: &str) -> Result<Policy, String> {
        serde_json::from_str(text).map_err(|e| format!("policy: {e}"))
    }

    /// The `features` bits of the switchable features the policy names.
    pub fn feature_bits(&self) -> u32 {
        self.features
            .iter()
            .filter_map(|f| FEATURES.iter().find(|(n, _)| n == f))
            .fold(0, |acc, (_, b)| acc | b)
    }

    /// Named features the engine cannot switch at the pin (the host reports them; WA-6 decides).
    pub fn unswitchable(&self) -> Vec<&str> {
        self.features
            .iter()
            .filter(|f| !FEATURES.iter().any(|(n, _)| n == f))
            .map(String::as_str)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_are_defaults_and_unknown_fields_are_refused() {
        let p = Policy::from_json(r#"{"origins":["https://a.example"]}"#).unwrap();
        assert_eq!(p.origins, ["https://a.example"]);
        assert_eq!(p.heap_mb, None);
        assert!(p.background_throttle);
        assert_eq!(p.telemetry_off, TelemetryOff::default());
        assert!(Policy::from_json("1").is_err());
        assert!(Policy::from_json(r#"{"origins":[],"patches":[]}"#).is_err());
        assert!(Policy::from_json(r#"{"telemetry_off":{"host":[]}}"#).is_err());
        assert!(Policy::from_json(r#"{"heap_mb":"big"}"#).is_err());
    }

    #[test]
    fn the_five_frozen_fields_round_trip() {
        let p = Policy {
            origins: vec!["https://*.example.com".into()],
            heap_mb: Some(512),
            telemetry_off: TelemetryOff {
                hosts: vec!["stats.example.com".into()],
                beacon_paths: vec!["/beacon".into()],
            },
            background_throttle: false,
            features: vec!["webgl2".into(), "midi".into()],
        };
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "background_throttle",
                "features",
                "heap_mb",
                "origins",
                "telemetry_off"
            ]
        );
        assert_eq!(Policy::from_json(&v.to_string()).unwrap(), p);
    }

    #[test]
    fn features_map_to_bits_and_the_rest_is_reported() {
        let p = Policy {
            features: vec!["webgl2".into(), "indexeddb".into(), "midi".into()],
            ..Policy::default()
        };
        assert_eq!(
            p.feature_bits(),
            crate::ffi::FEATURE_WEBGL2 | crate::ffi::FEATURE_INDEXEDDB
        );
        assert_eq!(p.unswitchable(), ["midi"]);
        assert_eq!(Policy::default().feature_bits(), 0);
    }
}
