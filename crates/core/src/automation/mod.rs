//! Assistants & Automation: what helpers (`wol mcp` clients, scripts) may do and what they did.
//! - [`policy`]: off switch + allow/deny per capability (`$XDG_CONFIG_HOME/wolos/helpers.json`).
//! - [`audit`]: every helper action (`$XDG_STATE_HOME/wolos/actions.jsonl`).
//! - [`tidy`]: Downloads tidy plan (dry run first, home only, undo through the journal).
//!
//! Helpers run as the user: the policy keeps well-behaved helpers inside what the user allowed; it
//! is not a sandbox. Administrator actions stay behind wol-admind and the password prompt.
pub mod audit;
pub mod policy;
pub mod tidy;

pub use audit::{HelperSeen, Outcome, Record};
pub use policy::{CAPABILITIES, Policy, Refusal};

/// One capability as the Settings pane shows it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityState {
    pub id: String,
    pub label: String,
    pub allowed: bool,
}

/// One Always Allow: `helper` changes `capability` without asking.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Grant {
    pub helper: String,
    pub capability: String,
    pub label: String,
}

/// Settings > Assistants & Automation data.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Status {
    pub enabled: bool,
    pub capabilities: Vec<CapabilityState>,
    /// Helpers seen in the audit log, most recent first.
    pub helpers: Vec<HelperSeen>,
    /// Always Allow choices, by helper then capability.
    #[serde(default)]
    pub always: Vec<Grant>,
}

pub fn status(policy: &Policy, records: &[Record]) -> Status {
    Status {
        enabled: policy.enabled,
        capabilities: CAPABILITIES
            .iter()
            .map(|(id, label)| CapabilityState {
                id: (*id).into(),
                label: (*label).into(),
                allowed: !policy.deny.iter().any(|d| d == id),
            })
            .collect(),
        helpers: audit::helpers(records),
        always: policy
            .always
            .iter()
            .flat_map(|(h, caps)| {
                caps.iter().filter_map(move |c| {
                    Some(Grant {
                        helper: h.clone(),
                        capability: c.clone(),
                        label: policy::label(c)?.into(),
                    })
                })
            })
            .collect(),
    }
}

/// Changes the helper policy, journaled as [`crate::journal::HELPERS`] (undo from Settings or the
/// CLI; a helper cannot undo it). Returns the journal entry id (None = nothing changed).
pub fn set_policy(
    j: &crate::journal::Journal,
    by: &str,
    now: u64,
    f: impl FnOnce(&mut Policy) -> Result<(), Refusal>,
) -> Result<Option<u64>, String> {
    let file = j.config_home().join(policy::FILE);
    let mut p = Policy::parse(std::fs::read_to_string(file).ok().as_deref());
    f(&mut p).map_err(|e| e.to_string())?;
    j.save(
        crate::journal::HELPERS,
        by,
        &[(policy::FILE, Some(&p.to_json()))],
        now,
    )
    .map_err(|e| e.to_string())
}

/// The off switch: helpers may act (true) or not at all.
pub fn set_enabled(
    j: &crate::journal::Journal,
    by: &str,
    on: bool,
    now: u64,
) -> Result<Option<u64>, String> {
    set_policy(j, by, now, |p| {
        p.enabled = on;
        Ok(())
    })
}

/// Allow or deny one capability ([`CAPABILITIES`] id).
pub fn set_capability(
    j: &crate::journal::Journal,
    by: &str,
    cap: &str,
    allowed: bool,
    now: u64,
) -> Result<Option<u64>, String> {
    set_policy(j, by, now, |p| p.set(cap, allowed))
}

/// Always Allow from the approval sheet: `helper` may change `cap` without asking.
pub fn allow_always(
    j: &crate::journal::Journal,
    by: &str,
    helper: &str,
    cap: &str,
    now: u64,
) -> Result<Option<u64>, String> {
    set_policy(j, by, now, |p| p.allow_always(helper, cap))
}

/// Reads the policy and audit log for the session user.
pub fn user_status() -> Status {
    let config = crate::theme::config_home();
    let policy = Policy::parse(
        std::fs::read_to_string(config.join(policy::FILE))
            .ok()
            .as_deref(),
    );
    status(&policy, &audit::read(&crate::journal::state_home()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lists_every_capability_and_seen_helpers() {
        let p = Policy {
            enabled: true,
            deny: vec!["files".into()],
            ..Policy::default()
        };
        let r = Record {
            t: 5,
            helper: "Claude".into(),
            via: "mcp".into(),
            tool: "appearance set".into(),
            args: vec!["dark".into()],
            result: Outcome::Ok,
            detail: String::new(),
        };
        let s = status(&p, &[r]);
        assert_eq!(s.capabilities.len(), CAPABILITIES.len());
        let files = s.capabilities.iter().find(|c| c.id == "files").unwrap();
        assert!(!files.allowed);
        assert!(
            s.capabilities
                .iter()
                .filter(|c| c.id != "files")
                .all(|c| c.allowed)
        );
        assert_eq!(s.helpers[0].name, "Claude");
        assert!(s.always.is_empty());
        let mut p = p;
        p.allow_always("Claude", "appearance").unwrap();
        assert_eq!(
            status(&p, &[]).always,
            [Grant {
                helper: "Claude".into(),
                capability: "appearance".into(),
                label: "Change appearance and wallpaper".into(),
            }]
        );
    }

    /// The approval sheet's Always Allow: journaled like every policy change.
    #[test]
    fn always_allow_is_journaled() {
        let home = std::env::temp_dir().join(format!("wol-auto-always-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let j = crate::journal::Journal::for_config(&home);
        assert_eq!(
            allow_always(&j, "settings", "Claude", "sound", 10),
            Ok(Some(1))
        );
        assert!(allow_always(&j, "settings", "Claude", "nope", 11).is_err());
        let text = std::fs::read_to_string(home.join(policy::FILE)).unwrap();
        assert!(Policy::parse(Some(&text)).always_allows("Claude", "sound"));
        assert!(
            j.undo(None, "mcp:Claude", 12).is_err(),
            "a helper cannot undo it"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn policy_changes_are_journaled_and_only_the_user_undoes_them() {
        let home = std::env::temp_dir().join(format!("wol-auto-policy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let j = crate::journal::Journal::for_config(&home);
        assert_eq!(
            set_capability(&j, "settings", "files", false, 10),
            Ok(Some(1))
        );
        assert_eq!(set_enabled(&j, "settings", false, 11), Ok(Some(2)));
        assert!(set_capability(&j, "settings", "rm", false, 12).is_err());
        let read = || {
            Policy::parse(
                std::fs::read_to_string(home.join(policy::FILE))
                    .ok()
                    .as_deref(),
            )
        };
        assert_eq!(
            read(),
            Policy {
                enabled: false,
                deny: vec!["files".into()],
                ..Policy::default()
            }
        );
        assert!(
            j.undo(None, "mcp:Claude", 13).is_err(),
            "a helper cannot undo it"
        );
        assert!(j.undo(None, "settings", 14).is_ok());
        assert!(read().enabled);
        assert!(
            home.join(".wol-state/wolos/journal.jsonl").exists(),
            "a temporary home never writes the user's journal"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
