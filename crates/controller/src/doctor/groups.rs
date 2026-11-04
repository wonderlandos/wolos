// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol doctor` security check for accounts made by an older WolOS (QA-FILES-2): every account
//! was put in `input` and `audio`, so any program of any account could read every keyboard (the
//! lock screen's and the login window's passwords included) and the microphone of the session at
//! the screen. New accounts are no longer added (`wol_core::settings::users::ACCOUNT_GROUPS`);
//! this names the ones still in them and the commands that take them out.
use wol_core::settings::users::RETIRED_GROUPS;

/// The human accounts (uid 1000..65533 in `passwd`) still in a retired group, as
/// (account, group) in `/etc/group` order, and the `gpasswd -d ACCOUNT GROUP` that removes each.
pub fn groups_migration(group: &str, passwd: &str) -> Vec<(String, &'static str, Vec<String>)> {
    let humans: Vec<String> = wol_core::settings::users::parse(passwd)
        .into_iter()
        .map(|a| a.name)
        .collect();
    let mut out = Vec::new();
    for line in group.lines() {
        let f: Vec<&str> = line.trim().split(':').collect();
        let (Some(name), Some(members)) = (f.first(), f.get(3)) else {
            continue;
        };
        let Some(g) = RETIRED_GROUPS.iter().find(|g| *g == name) else {
            continue;
        };
        for m in members.split(',').map(str::trim).filter(|m| !m.is_empty()) {
            if humans.iter().any(|h| h == m) {
                let argv = ["gpasswd", "-d", m, g].map(String::from).to_vec();
                out.push((m.to_string(), *g, argv));
            }
        }
    }
    out
}

/// The check: Ok when no account is in a retired group, else Warn with the accounts named and
/// the commands for the "Show the command" line (no button: wol-admind has no group action).
pub fn check(group: Option<&str>, passwd: Option<&str>) -> super::Check {
    let (Some(group), Some(passwd)) = (group, passwd) else {
        return super::check(
            "security",
            "keyboard-access",
            "Keyboard access",
            super::Status::Skipped,
            "The account and group lists could not be read.",
        );
    };
    let found = groups_migration(group, passwd);
    if found.is_empty() {
        return super::check(
            "security",
            "keyboard-access",
            "Keyboard access",
            super::Status::Ok,
            "Only the session at the screen can read the keyboard and the microphone.",
        );
    }
    let mut names: Vec<&str> = Vec::new();
    for (n, _, _) in &found {
        if !names.contains(&n.as_str()) {
            names.push(n);
        }
    }
    let commands: Vec<String> = found.iter().map(|(_, _, a)| a.join(" ")).collect();
    super::check(
        "security",
        "keyboard-access",
        "Keyboard access",
        super::Status::Warn,
        &format!(
            "Programs of {} can read every keyboard and microphone, also while someone else is \
             signed in. An administrator can remove that access in a terminal: sudo {}",
            names.join(", "),
            commands.join("; sudo ")
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0::/root:/bin/bash\n\
        alice:x:1000:1000:Alice:/home/alice:/usr/bin/zsh\n\
        bob:x:1001:1001:Bob:/home/bob:/usr/bin/zsh\n\
        rtkit:x:133:133::/proc:/usr/bin/nologin\n";

    /// QA-FILES-2: accounts an older release put in input/audio are named with their command;
    /// system members (rtkit in audio) and other groups are left alone.
    #[test]
    fn names_accounts_in_retired_groups() {
        let group = "wheel:x:998:alice\ninput:x:97:alice,bob\naudio:x:995:rtkit,alice\nvideo:x:985:alice,bob\n";
        let found = groups_migration(group, PASSWD);
        let cmds: Vec<String> = found.iter().map(|(_, _, a)| a.join(" ")).collect();
        assert_eq!(
            cmds,
            [
                "gpasswd -d alice input",
                "gpasswd -d bob input",
                "gpasswd -d alice audio"
            ]
        );
        let c = check(Some(group), Some(PASSWD));
        assert_eq!(c.status, super::super::Status::Warn);
        assert!(c.detail.contains("alice, bob"), "{}", c.detail);
        assert!(
            c.detail.contains("sudo gpasswd -d alice input"),
            "{}",
            c.detail
        );
        let clean = "input:x:97:\naudio:x:995:rtkit\n";
        assert!(groups_migration(clean, PASSWD).is_empty());
        assert_eq!(
            check(Some(clean), Some(PASSWD)).status,
            super::super::Status::Ok
        );
        assert_eq!(
            check(None, Some(PASSWD)).status,
            super::super::Status::Skipped
        );
    }
}
