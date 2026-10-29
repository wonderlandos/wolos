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

    pub fn gtk_theme(mode: &str) -> &'static str {
        if mode == DARK {
            "WhiteSur-Dark"
        } else {
            "WhiteSur-Light"
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
    /// Groups of every account: seat (the display, through seatd), video (cameras, GPU render nodes).
    pub const ACCOUNT_GROUPS: &str = "seat,video,input,audio";
    /// Administrators, like the Setup Assistant's account: wheel (sudo via /etc/sudoers.d,
    /// wol-admind). USB sticks mount through udisks for every session.
    pub const ACCOUNT_GROUPS_ADMIN: &str = "wheel,seat,video,input,audio";

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
    const RESERVED: [&str; 12] = [
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
    ];
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
            (
                "name",
                is_valid_name(&a.name) && !RESERVED.contains(&a.name.as_str()),
            ),
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

    /// `useradd -m -c NAME -s zsh -G groups LOGIN`.
    pub fn add_argv(name: &str, full_name: &str, admin: bool) -> Option<Vec<String>> {
        if !is_valid_name(name) {
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

    /// New account password: 8..=128 characters, no control characters.
    pub fn is_valid_password(p: &str) -> bool {
        (8..=128).contains(&p.chars().count()) && !p.chars().any(char::is_control)
    }

    /// `chpasswd`: `NAME:PASSWORD` read from stdin (never argv).
    pub fn password_argv(name: &str) -> Option<Vec<String>> {
        is_valid_name(name).then(|| vec!["chpasswd".into()])
    }

    pub fn remove_argv(name: &str) -> Option<Vec<String>> {
        is_valid_name(name).then(|| ["userdel", "-r", name].map(String::from).to_vec())
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
            "useradd -m -c Bb -s /usr/bin/zsh -G wheel,seat,video,input,audio bob"
        );
        // Admins match the Setup Assistant's account; everyone gets cameras (group video).
        assert_eq!(
            users::add_argv("ann", "Ann", false).unwrap().join(" "),
            "useradd -m -c Ann -s /usr/bin/zsh -G seat,video,input,audio ann"
        );
        assert!(users::remove_argv("x y").is_none());
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
