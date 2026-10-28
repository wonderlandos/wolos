//! What `wol games steam` tells someone who opened Steam from the Dock, Launchpad or Spotlight
//! (no terminal): one notification, title + body. Bodies follow HIG §3 and pass
//! `shell::copy::lint` (crates/shell/tests/copy_lint.rs).
use super::doctor::Check;

/// (title, body) of one notification.
pub type Notice = (&'static str, &'static str);

/// First start: the client downloads itself; its own updater window then shows progress.
pub const PREPARING: Notice = (
    "Getting Steam ready",
    "Steam downloads itself the first time you open it. It opens when it's ready.",
);

/// The client could not be downloaded.
pub const NO_DOWNLOAD: Notice = (
    "Steam couldn't download",
    "Check that you're online, then open Steam again.",
);

/// The graphics driver is not loaded.
pub const SETTINGS: Notice = (
    "Steam can't open yet",
    "Your graphics driver isn't running. Restart your computer, then open Steam again.",
);

/// A package the client needs is not installed (the doctor names it).
pub const MISSING: Notice = (
    "Steam can't open yet",
    "Part of Steam is missing. To add it, open Terminal and type wol games doctor.",
);

/// The 32-bit NVIDIA libraries (the Steam client draws with them) are missing.
pub const NVIDIA: Notice = (
    "Steam can't open yet",
    "Steam needs graphics files for your NVIDIA card. To add them, open Terminal and type wol games doctor.",
);

pub const ALL: [Notice; 5] = [PREPARING, NO_DOWNLOAD, SETTINGS, MISSING, NVIDIA];

/// The notice for what keeps the client from starting ([`super::doctor::steam_blockers`]).
pub fn blocked(blockers: &[&Check]) -> Notice {
    let has = |id: &str| blockers.iter().any(|c| c.id == id);
    if has("steam") || has("multilib") || has("lib32-gl") {
        MISSING
    } else if has("nvidia") {
        NVIDIA
    } else {
        SETTINGS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::doctor::Status;

    fn fail(id: &'static str) -> Check {
        Check {
            id,
            status: Status::Fail,
            detail: String::new(),
            fix: String::new(),
        }
    }

    #[test]
    fn each_blocker_gets_its_notice() {
        for (ids, want) in [
            (&["drm"][..], SETTINGS),
            (&["steam"][..], MISSING),
            (&["multilib", "lib32-gl"][..], MISSING),
            (&["nvidia"][..], NVIDIA),
            (&["nvidia", "steam"][..], MISSING),
        ] {
            let checks: Vec<Check> = ids.iter().map(|i| fail(i)).collect();
            let refs: Vec<&Check> = checks.iter().collect();
            assert_eq!(blocked(&refs), want, "{ids:?}");
        }
    }

    /// Titles are labels: short, capitalised, no full stop.
    #[test]
    fn titles_are_labels() {
        for (t, _) in ALL {
            assert!(t.len() <= 32 && t.starts_with(char::is_uppercase), "{t}");
            assert!(!t.ends_with('.'), "{t}");
        }
    }
}
