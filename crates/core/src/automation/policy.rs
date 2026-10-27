//! Helper policy: `$XDG_CONFIG_HOME/wolos/helpers.json` =
//! `{"enabled": true, "deny": ["files"], "always": {"Claude": ["appearance"]}}`.
//! No file = on, nothing denied (connecting a helper is the opt-in). Malformed = off (fail closed,
//! as the compositor consent database). Unknown capabilities are never allowed.
//! A change asks on screen unless the user chose Always Allow for that helper and capability
//! (`always`); administrator changes ask every time. Helper names are what the helper says it is.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Relative to the config home.
pub const FILE: &str = "wolos/helpers.json";

/// (id, Settings label). `screen-capture` and `window-list` are the compositor capability names.
pub const CAPABILITIES: [(&str, &str); 12] = [
    ("read", "See status and settings"),
    ("appearance", "Change appearance and wallpaper"),
    ("sound", "Change the volume"),
    ("display", "Change brightness"),
    ("network", "Turn Wi-Fi and Bluetooth on or off"),
    ("focus", "Turn Do Not Disturb on or off"),
    ("apps", "Open apps, files and links"),
    ("notify", "Show notifications"),
    ("screen-capture", "Take screenshots"),
    ("window-list", "See and switch windows"),
    ("files", "Tidy the Downloads folder"),
    ("restore", "Undo changes and pick a restore point"),
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub enabled: bool,
    #[serde(default)]
    pub deny: Vec<String>,
    /// Helper name -> capabilities it may change without asking (Always Allow), sorted.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub always: BTreeMap<String, Vec<String>>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            deny: Vec::new(),
            always: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Off,
    Denied(&'static str),
    Unknown(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => write!(
                f,
                "Helpers are turned off on this computer. You can turn them on in Settings › Assistants & Automation."
            ),
            Self::Denied(label) => write!(
                f,
                "Helpers are not allowed to: {label}. You can change this in Settings › Assistants & Automation."
            ),
            Self::Unknown(c) => write!(f, "unknown capability '{c}'"),
        }
    }
}

pub fn label(cap: &str) -> Option<&'static str> {
    CAPABILITIES.iter().find(|c| c.0 == cap).map(|c| c.1)
}

impl Policy {
    pub fn parse(text: Option<&str>) -> Self {
        match text {
            None => Self::default(),
            Some(t) => serde_json::from_str(t).unwrap_or(Self {
                enabled: false,
                ..Self::default()
            }),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    pub fn check(&self, cap: &str) -> Result<(), Refusal> {
        let Some(l) = label(cap) else {
            return Err(Refusal::Unknown(cap.into()));
        };
        if !self.enabled {
            Err(Refusal::Off)
        } else if self.deny.iter().any(|d| d == cap) {
            Err(Refusal::Denied(l))
        } else {
            Ok(())
        }
    }

    /// Allow or deny one capability; unknown names are refused. Denying also takes back every
    /// Always Allow for it.
    pub fn set(&mut self, cap: &str, allowed: bool) -> Result<(), Refusal> {
        label(cap).ok_or_else(|| Refusal::Unknown(cap.into()))?;
        self.deny.retain(|d| d != cap);
        if !allowed {
            self.deny.push(cap.into());
            self.deny.sort();
            self.always
                .values_mut()
                .for_each(|c| c.retain(|x| x != cap));
            self.always.retain(|_, c| !c.is_empty());
        }
        Ok(())
    }

    /// `helper` may change `cap` without asking: helpers on, capability allowed, granted.
    pub fn always_allows(&self, helper: &str, cap: &str) -> bool {
        self.check(cap).is_ok()
            && self
                .always
                .get(helper)
                .is_some_and(|c| c.iter().any(|x| x == cap))
    }

    /// Always Allow on the approval sheet.
    pub fn allow_always(&mut self, helper: &str, cap: &str) -> Result<(), Refusal> {
        label(cap).ok_or_else(|| Refusal::Unknown(cap.into()))?;
        let c = self.always.entry(helper.into()).or_default();
        if !c.iter().any(|x| x == cap) {
            c.push(cap.into());
            c.sort();
        }
        Ok(())
    }

    /// Takes back every Always Allow of `helper`.
    pub fn forget(&mut self, helper: &str) {
        self.always.remove(helper);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_fail_closed_and_checks_table() {
        assert_eq!(Policy::parse(None), Policy::default());
        assert!(!Policy::parse(Some("{not json")).enabled, "malformed = off");
        let p = Policy::parse(Some(r#"{"enabled":true,"deny":["files"]}"#));
        for (cap, ok) in [
            ("appearance", true),
            ("read", true),
            ("files", false),
            ("rm-rf", false),
        ] {
            assert_eq!(p.check(cap).is_ok(), ok, "{cap}");
        }
        assert_eq!(
            p.check("files"),
            Err(Refusal::Denied("Tidy the Downloads folder"))
        );
        let off = Policy {
            enabled: false,
            ..Policy::default()
        };
        assert_eq!(off.check("read"), Err(Refusal::Off));
    }

    #[test]
    fn set_round_trips_and_refuses_unknown() {
        let mut p = Policy::default();
        p.set("sound", false).unwrap();
        p.set("files", false).unwrap();
        p.set("sound", false).unwrap();
        assert_eq!(p.deny, ["files", "sound"]);
        p.set("sound", true).unwrap();
        assert_eq!(Policy::parse(Some(&p.to_json())), p);
        assert!(p.set("everything", true).is_err());
        let mut ids: Vec<_> = CAPABILITIES.iter().map(|c| c.0).collect();
        ids.dedup();
        assert_eq!(ids.len(), CAPABILITIES.len());
    }

    /// Always Allow: per helper and capability; deny and forget take it back; old files still parse.
    #[test]
    fn always_allow_grants() {
        let mut p = Policy::parse(Some(r#"{"enabled":true,"deny":[]}"#));
        assert!(!p.always_allows("Claude", "appearance"));
        p.allow_always("Claude", "appearance").unwrap();
        p.allow_always("Claude", "appearance").unwrap();
        p.allow_always("Claude", "sound").unwrap();
        p.allow_always("script:backup", "files").unwrap();
        assert!(p.always_allows("Claude", "appearance"));
        assert!(!p.always_allows("llama", "appearance"), "per helper");
        assert_eq!(p.always["Claude"], ["appearance", "sound"]);
        assert!(p.allow_always("Claude", "rm-rf").is_err());
        assert_eq!(Policy::parse(Some(&p.to_json())), p);
        p.set("sound", false).unwrap();
        assert!(!p.always_allows("Claude", "sound"), "deny revokes grants");
        p.forget("Claude");
        assert!(!p.always.contains_key("Claude"));
        assert!(p.always_allows("script:backup", "files"));
        p.enabled = false;
        assert!(!p.always_allows("script:backup", "files"), "off = off");
        assert!(
            !Policy::default().to_json().contains("always"),
            "no grants, same file as before"
        );
    }
}
