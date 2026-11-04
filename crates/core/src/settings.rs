// SPDX-License-Identifier: BSD-2-Clause
//! System settings files edited by wol-admind: appearance, keybinds, idle, users, time zones.
use std::collections::BTreeMap;

pub mod appearance {
    pub const LIGHT: &str = "light";
    pub const DARK: &str = "dark";
    pub const FILE: &str = "/etc/wolos/appearance";

    /// Anything but "dark" reads as light.
    pub fn parse(text: Option<&str>) -> &'static str {
        if text.map(str::trim) == Some(DARK) {
            DARK
        } else {
            LIGHT
        }
    }
}

pub mod keybinds {
    use super::BTreeMap;

    pub const FILE: &str = "/etc/wolos/keybinds.json";
    const MODS: [&str; 9] = [
        "super", "cmd", "logo", "mod4", "ctrl", "control", "alt", "mod1", "shift",
    ];

    pub fn parse(json: &str) -> Option<BTreeMap<String, Vec<String>>> {
        serde_json::from_str(json).ok()
    }

    /// Known modifiers, then one non-empty key name.
    pub fn is_valid_combo(combo: &str) -> bool {
        let parts: Vec<&str> = combo.split('+').map(str::trim).collect();
        let (key, mods) = parts.split_last().expect("split yields one");
        !key.is_empty()
            && mods
                .iter()
                .all(|m| MODS.contains(&m.to_ascii_lowercase().as_str()))
    }

    /// Canonical sorted, indented form.
    pub fn to_json(binds: &BTreeMap<String, Vec<String>>) -> String {
        serde_json::to_string_pretty(binds).unwrap_or_default()
    }
}

pub mod idle {
    pub const FILE: &str = "/etc/wolos/idle.json";
    pub const DEFAULT_SECS: u32 = 300;

    pub fn parse(json: &str) -> u32 {
        serde_json::from_str::<serde_json::Value>(json)
            .ok()
            .and_then(|v| v.get("lock_after_secs")?.as_u64())
            .map_or(DEFAULT_SECS, |s| s.min(u64::from(u32::MAX)) as u32)
    }

    pub fn to_json(secs: u32) -> String {
        format!("{{\"lock_after_secs\":{secs}}}")
    }
}

pub mod users {
    /// Login shell of every account WolOS creates (Setup Assistant, Settings > Accounts, live user).
    /// The zsh package registers it in /etc/shells. root keeps bash.
    pub const LOGIN_SHELL: &str = "/usr/bin/zsh";

    /// Caps the btrfs subvolume under /home at 85 % of the pool (`files/usr/lib/wolos/home-quota`,
    /// ADR 0023); run by `install-run` for the first account's /home and after every `useradd`.
    pub const HOME_QUOTA: &str = "/usr/lib/wolos/home-quota";
    /// Groups of every account: seat (the display, through seatd), video (cameras, GPU render nodes).
    /// Never `input` or `audio` (QA-FILES-2): `input` opens every /dev/input/event* to any process
    /// of the account, so a background program of one person read the keys typed at the lock screen
    /// and the login window; `audio` opened the microphone to every session, not only the one at
    /// the screen. The session at the screen gets both from logind (uaccess) and seatd.
    pub const ACCOUNT_GROUPS: &str = "seat,video";
    /// Administrators, like the Setup Assistant's account: wheel (sudo via /etc/sudoers.d,
    /// wol-admind). USB sticks mount through udisks for every session.
    pub const ACCOUNT_GROUPS_ADMIN: &str = "wheel,seat,video";
    /// Groups older WolOS releases gave every account; `wol doctor` names the accounts still in
    /// them (controller doctor::groups).
    pub const RETIRED_GROUPS: [&str; 2] = ["input", "audio"];

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Account {
        pub name: String,
        pub uid: u32,
        pub full_name: String,
        pub home: String,
    }

    const NO_LOGIN: [&str; 4] = [
        "/usr/sbin/nologin",
        "/sbin/nologin",
        "/bin/false",
        "/nonexistent",
    ];

    /// Human accounts: uid 1000..65533 with a login shell.
    pub fn parse(passwd: &str) -> Vec<Account> {
        passwd
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| {
                let f: Vec<&str> = l.split(':').collect();
                let uid: u32 = f.get(2)?.parse().ok()?;
                (f.len() >= 7 && (1000..65534).contains(&uid) && !NO_LOGIN.contains(&f[6])).then(
                    || Account {
                        name: f[0].into(),
                        uid,
                        full_name: f[4].split(',').next().unwrap_or("").into(),
                        home: f[5].into(),
                    },
                )
            })
            .collect()
    }

    /// `^[a-z_][a-z0-9_-]{0,31}$`
    pub fn is_valid_name(n: &str) -> bool {
        let b = n.as_bytes();
        !b.is_empty()
            && b.len() <= 32
            && (b[0].is_ascii_lowercase() || b[0] == b'_')
            && b[1..]
                .iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
    }

    /// Setup assistant / wol-login account rules (ADR 0004).
    const MAX_PASSWORD: usize = 512;
    /// Names an account can never take: WolOS's own (`live` is the installer session, `wolos` the
    /// product, `_wollogin` and `_xguard` service users) and the Arch system users (`systemd-*`
    /// covers systemd-network, -resolve, -timesync, -journal-remote, -coredump, -oom).
    const RESERVED: [&str; 23] = [
        "root",
        "toor",
        "daemon",
        "operator",
        "bin",
        "nobody",
        "www",
        "sshd",
        "games",
        "news",
        "man",
        "_wollogin",
        "_xguard",
        "live",
        "wolos",
        "http",
        "mail",
        "ftp",
        "dbus",
        "polkitd",
        "rtkit",
        "seatd",
        "git",
    ];

    /// A reserved account name ([`RESERVED`], or any `systemd-*` user).
    pub fn is_reserved(name: &str) -> bool {
        RESERVED.contains(&name) || name.starts_with("systemd-")
    }
    pub const MAX_FIELD: usize = 64;

    fn plain(s: &str) -> bool {
        s.chars().count() <= MAX_FIELD && !s.chars().any(|c| c.is_control() || c == ':')
    }

    /// Which field is wrong, if any.
    pub fn validate_account(a: &shared::login::NewAccount) -> Result<(), &'static str> {
        let locale = a
            .locale
            .strip_suffix(".UTF-8")
            .and_then(|l| l.split_once('_'));
        let checks = [
            ("name", is_valid_name(&a.name) && !is_reserved(&a.name)),
            ("full name", plain(&a.full_name)),
            ("hint", plain(&a.hint) && a.hint != a.password),
            (
                "locale",
                locale.is_some_and(|(l, r)| {
                    (2..=3).contains(&l.len())
                        && l.bytes().all(|b| b.is_ascii_lowercase())
                        && r.len() == 2
                        && r.bytes().all(|b| b.is_ascii_uppercase())
                }),
            ),
            (
                "keymap",
                (2..=16).contains(&a.keymap.len())
                    && a.keymap
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b"()".contains(&b)),
            ),
            (
                "appearance",
                ["light", "dark", "auto"].contains(&a.appearance.as_str()),
            ),
            (
                "password",
                !a.password.is_empty()
                    && a.password.len() <= MAX_PASSWORD
                    && !a.password.contains('\0'),
            ),
        ];
        match checks.iter().find(|(_, ok)| !ok) {
            Some((why, _)) => Err(why),
            None => Ok(()),
        }
    }

    /// `useradd -m -c NAME -s zsh -G groups LOGIN`; None for an invalid or reserved name.
    pub fn add_argv(name: &str, full_name: &str, admin: bool) -> Option<Vec<String>> {
        if !is_valid_name(name) || is_reserved(name) {
            return None;
        }
        // Admins get the Setup Assistant account's groups (controller::login::setup_plan).
        let groups = if admin {
            ACCOUNT_GROUPS_ADMIN
        } else {
            ACCOUNT_GROUPS
        };
        Some(
            [
                "useradd",
                "-m",
                "-c",
                &full_name.replace(':', ""),
                "-s",
                LOGIN_SHELL,
                "-G",
                groups,
                name,
            ]
            .map(String::from)
            .to_vec(),
        )
    }

    /// Longest account name `useradd` takes.
    pub const NAME_MAX: usize = 32;

    /// Latin letters with marks and ligatures as plain ASCII ("Ñandú" -> "nandu"). Other scripts
    /// give nothing: the person types the account name (QA-FILES-16: only ASCII was kept, so
    /// "Ñandú Álvarez" became `and`).
    pub fn ascii_of(c: char) -> &'static str {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
            'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
            'ď' | 'đ' | 'ð' => "d",
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
            'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
            'ĥ' | 'ħ' => "h",
            'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
            'ĵ' => "j",
            'ķ' => "k",
            'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
            'ñ' | 'ń' | 'ņ' | 'ň' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
            'ŕ' | 'ŗ' | 'ř' => "r",
            'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => "s",
            'ţ' | 'ť' | 'ŧ' | 'ț' => "t",
            'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
            'ŵ' => "w",
            'ý' | 'ÿ' | 'ŷ' => "y",
            'ź' | 'ż' | 'ž' => "z",
            'ß' => "ss",
            'æ' => "ae",
            'œ' => "oe",
            'þ' => "th",
            _ => "",
        }
    }

    /// The letters of `full` as account-name letters, before the [`NAME_MAX`] cut: lowercase,
    /// transliterated ([`ascii_of`]), spaces and punctuation dropped, leading digits dropped (a
    /// name starts with a letter).
    pub fn derive_letters(full: &str) -> String {
        let mut s = String::new();
        for c in full.to_lowercase().chars() {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                s.push(c)
            } else {
                s.push_str(ascii_of(c))
            }
        }
        s.trim_start_matches(|c: char| c.is_ascii_digit())
            .to_string()
    }

    /// The account name WolOS suggests for a full name, one rule for the Setup Assistant and
    /// Settings > Users > Add User (QA-LIVE3-24: Setup made `maracosta` of "Mara Costa", Add User
    /// `tomas` of "Tomás Reis"): the whole name, transliterated, at most [`NAME_MAX`] letters.
    /// "Alice Liddell" -> "aliceliddell", "Ñandú Álvarez" -> "nandualvarez"; "" when nothing
    /// Latin is left.
    pub fn derive_name(full: &str) -> String {
        derive_letters(full).chars().take(NAME_MAX).collect()
    }

    /// New account password: 8..=128 characters, no control characters.
    pub fn is_valid_password(p: &str) -> bool {
        (8..=128).contains(&p.chars().count()) && !p.chars().any(char::is_control)
    }

    /// `chpasswd`: `NAME:PASSWORD` read from stdin (never argv).
    pub fn password_argv(name: &str) -> Option<Vec<String>> {
        is_valid_name(name).then(|| vec!["chpasswd".into()])
    }

    /// Removes the account and keeps its home folder (QA-DATA-8: `userdel -r` deleted /home/NAME
    /// at once, with no word and no way back). [`remove_with_files_argv`] is the explicit choice.
    pub fn remove_argv(name: &str) -> Option<Vec<String>> {
        is_valid_name(name).then(|| ["userdel", name].map(String::from).to_vec())
    }

    /// Removes the account and its files (home folder, mail spool): only when the person chose so.
    pub fn remove_with_files_argv(name: &str) -> Option<Vec<String>> {
        is_valid_name(name).then(|| ["userdel", "-r", name].map(String::from).to_vec())
    }
}

/// The computer name (audit3 R2B-18): one rule for the Install app, `wol install` (plan and
/// autoinstall), Settings > Sharing and wol-admind's Hostname action.
pub mod hostname {
    /// The name WolOS uses when none is given or the given one is not valid.
    pub const DEFAULT: &str = "wolos";

    /// One RFC 1123 label, lowercase: 1..=63 of `[a-z0-9-]`, no `-` at either end. False for
    /// anything else (dots, capitals, spaces, shell characters, non-ASCII).
    pub fn is_valid(s: &str) -> bool {
        (1..=63).contains(&s.len())
            && !s.starts_with('-')
            && !s.ends_with('-')
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    }
}

pub mod timezones {
    pub fn is_valid(z: &str) -> bool {
        !z.is_empty()
            && z.len() < 64
            && z.as_bytes()[0].is_ascii_uppercase()
            && !z.contains("..")
            && z.chars()
                .all(|c| c.is_ascii_alphanumeric() || "/_-+".contains(c))
    }

    pub fn current(link_target: Option<&str>) -> &str {
        link_target
            .and_then(|t| t.find("zoneinfo/").map(|i| &t[i + 9..]))
            .unwrap_or("UTC")
    }

    pub fn set_argv(zone: &str) -> Vec<String> {
        vec!["timedatectl".into(), "set-timezone".into(), zone.into()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos() {
        for (c, ok) in [
            ("super+t", true),
            ("Super+Shift+4", true),
            ("Print", true),
            ("hyper+x", false),
            ("super+", false),
        ] {
            assert_eq!(keybinds::is_valid_combo(c), ok, "{c}");
        }
    }

    #[test]
    fn idle_and_appearance() {
        assert_eq!(idle::parse(r#"{"lock_after_secs":60}"#), 60);
        assert_eq!(idle::parse("junk"), idle::DEFAULT_SECS);
        assert_eq!(idle::parse(&idle::to_json(0)), 0);
        assert_eq!(appearance::parse(Some("dark\n")), "dark");
        assert_eq!(appearance::parse(None), "light");
    }

    #[test]
    fn users_table() {
        let p = "root:*:0:0:Charlie &:/root:/bin/sh\nklee:*:1001:1001:Klee,,,:/home/klee:/bin/sh\nsvc:*:1002:1:x:/:/usr/sbin/nologin\n";
        let a = users::parse(p);
        assert_eq!(a.len(), 1);
        assert_eq!(
            (a[0].name.as_str(), a[0].full_name.as_str()),
            ("klee", "Klee")
        );
        for (n, ok) in [
            ("bob", true),
            ("_x-1", true),
            ("-G", false),
            ("Bob", false),
            ("bob;id", false),
            ("", false),
        ] {
            assert_eq!(users::is_valid_name(n), ok, "{n}");
        }
        assert_eq!(
            users::add_argv("bob", "B:b", true).unwrap().join(" "),
            "useradd -m -c Bb -s /usr/bin/zsh -G wheel,seat,video bob"
        );
        // Admins match the Setup Assistant's account; everyone gets cameras (group video).
        assert_eq!(
            users::add_argv("ann", "Ann", false).unwrap().join(" "),
            "useradd -m -c Ann -s /usr/bin/zsh -G seat,video ann"
        );
        assert!(users::remove_argv("x y").is_none());
        // Audit S4-6: reserved names never reach useradd.
        for n in ["git", "live", "wolos", "systemd-x", "root"] {
            assert!(users::add_argv(n, "X", false).is_none(), "{n}");
        }
    }

    /// QA-LIVE3-24: one rule for every place that suggests an account name.
    #[test]
    fn derive_name_same_in_installer_and_adduser() {
        for (full, want) in [
            ("Mara Costa", "maracosta"),
            ("Tomás Reis", "tomasreis"),
            ("Alice Liddell", "aliceliddell"),
            ("  José  Álvarez-Núñez ", "josealvareznunez"),
            ("Ñandú Álvarez", "nandualvarez"),
            ("Weißmüller", "weissmuller"),
            ("42 Wonder", "wonder"),
            ("李小龍", ""),
            ("", ""),
        ] {
            assert_eq!(users::derive_name(full), want, "{full}");
            let n = users::derive_name(full);
            assert!(n.is_empty() || users::is_valid_name(&n), "{n}");
        }
        assert_eq!(users::derive_name(&"a".repeat(40)).len(), users::NAME_MAX);
    }

    /// QA-DATA-8: removing an account keeps its files unless deleting them was asked for.
    #[test]
    fn user_remove_keeps_home_by_default() {
        assert_eq!(users::remove_argv("bob").unwrap(), ["userdel", "bob"]);
        assert_eq!(
            users::remove_with_files_argv("bob").unwrap(),
            ["userdel", "-r", "bob"]
        );
        assert!(users::remove_with_files_argv("-r").is_none());
    }

    /// QA-FILES-2: no account WolOS creates can read the keyboard or the microphone of another
    /// session: neither `input` nor `audio` in any group list or useradd line.
    #[test]
    fn account_groups_have_no_input_audio() {
        for admin in [false, true] {
            let argv = users::add_argv("ann", "Ann", admin).unwrap();
            let groups = &argv[argv.iter().position(|a| a == "-G").unwrap() + 1];
            let groups: Vec<&str> = groups.split(',').collect();
            for g in users::RETIRED_GROUPS {
                assert!(!groups.contains(&g), "admin={admin}: {groups:?}");
            }
            assert!(
                groups.contains(&"seat") && groups.contains(&"video"),
                "{groups:?}"
            );
            assert_eq!(groups.contains(&"wheel"), admin, "{groups:?}");
        }
        for list in [users::ACCOUNT_GROUPS, users::ACCOUNT_GROUPS_ADMIN] {
            for g in users::RETIRED_GROUPS {
                assert!(!list.split(',').any(|x| x == g), "{list}");
            }
        }
    }

    /// The system's own users can never be chosen as an account (the live session's `live`, the
    /// Arch daemons' users, every `systemd-*`); ordinary names stay free.
    #[test]
    fn reserved_names() {
        for n in [
            "root",
            "live",
            "wolos",
            "_xguard",
            "_wollogin",
            "http",
            "mail",
            "ftp",
            "dbus",
            "polkitd",
            "rtkit",
            "seatd",
            "git",
            "nobody",
            "systemd-network",
            "systemd-resolve",
            "systemd-timesync",
            "systemd-oom",
        ] {
            assert!(users::is_reserved(n), "{n}");
        }
        for n in ["alice", "klee", "live2", "systemd", "wolos-fan", "mailman"] {
            assert!(!users::is_reserved(n), "{n}");
        }
        let a = shared::login::NewAccount {
            name: "live".into(),
            ..Default::default()
        };
        assert_eq!(users::validate_account(&a), Err("name"));
    }

    #[test]
    fn zones() {
        for (z, ok) in [
            ("America/Sao_Paulo", true),
            ("UTC", true),
            ("../../etc/passwd", false),
            ("/etc/passwd", false),
            ("-r", false),
        ] {
            assert_eq!(timezones::is_valid(z), ok, "{z}");
        }
        assert_eq!(
            timezones::current(Some("/usr/share/zoneinfo/Europe/Lisbon")),
            "Europe/Lisbon"
        );
        assert_eq!(timezones::current(None), "UTC");
    }
}
