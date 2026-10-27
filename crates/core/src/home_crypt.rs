//! "Encrypt my files": the account's home is its own ZFS dataset (`zroot/home/<name>`), encrypted
//! with AES-256-GCM under a key ZFS derives from the login password (keyformat=passphrase,
//! PBKDF2). At boot it stays locked (canmount=noauto, no key); the login window unlocks it with
//! the password it has just checked, and logging out locks it again when nothing still uses it.
//! Off by default until it is proven on hardware (archicture/modules/security.md).
//!
//! Pure: commands for the caller to run (wol-login, the Setup Assistant plan), passwords on stdin.

/// Parent dataset of every home (the installer's pool and layout, image/installerconfig).
pub const HOMES: &str = "zroot/home";
/// ZFS passphrase limits in bytes (account passwords are 8..=128 characters).
pub const MIN_PASSWORD: usize = 8;
pub const MAX_PASSWORD: usize = 512;

/// One command; `stdin` carries a password (never an argument: `ps` shows arguments).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    pub argv: Vec<String>,
    pub stdin: Option<String>,
}

fn cmd(argv: &[&str], stdin: Option<&str>) -> Cmd {
    Cmd {
        argv: argv.iter().map(|s| (*s).to_owned()).collect(),
        stdin: stdin.map(|p| format!("{p}\n")),
    }
}

/// `zroot/home/<name>`; None for a name WolOS would not create.
pub fn dataset(name: &str) -> Option<String> {
    crate::settings::users::is_valid_name(name).then(|| format!("{HOMES}/{name}"))
}

/// A password ZFS accepts as the passphrase (no newline: stdin is read up to one).
pub fn password_ok(password: &str) -> bool {
    (MIN_PASSWORD..=MAX_PASSWORD).contains(&password.len()) && !password.contains('\n')
}

/// What [`status_argv`] reports for one home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub encrypted: bool,
    pub key_loaded: bool,
    pub mounted: bool,
}

/// `zfs get` of encryption, keystatus and mounted; fails (None) when the home is not a dataset.
pub fn status_argv(name: &str) -> Option<Vec<String>> {
    let ds = dataset(name)?;
    Some(
        [
            "zfs",
            "get",
            "-H",
            "-o",
            "value",
            "encryption,keystatus,mounted",
            &ds,
        ]
        .map(String::from)
        .to_vec(),
    )
}

/// None: no dataset of its own (a plain folder, nothing to unlock).
pub fn parse_status(out: Option<&str>) -> Option<State> {
    let v: Vec<&str> = out?.lines().map(str::trim).collect();
    let [enc, key, mounted] = v[..] else {
        return None;
    };
    Some(State {
        encrypted: enc != "off",
        key_loaded: key == "available",
        mounted: mounted == "yes",
    })
}

/// Before `pw useradd -m`: the encrypted dataset, mounted at /home/<name>, so pw fills it.
pub fn create(name: &str, password: &str) -> Option<Vec<Cmd>> {
    let ds = dataset(name)?;
    if !password_ok(password) {
        return None;
    }
    let mountpoint = format!("mountpoint=/home/{name}");
    Some(vec![
        cmd(
            &[
                "zfs",
                "create",
                "-o",
                "encryption=aes-256-gcm",
                "-o",
                "keyformat=passphrase",
                "-o",
                "keylocation=prompt",
                "-o",
                "canmount=noauto",
                "-o",
                &mountpoint,
                &ds,
            ],
            Some(password),
        ),
        cmd(&["zfs", "mount", &ds], None),
    ])
}

/// After `pw useradd -m`: the folder belongs to the account and only to it.
pub fn after_add(name: &str) -> Option<Vec<Cmd>> {
    dataset(name)?;
    let home = format!("/home/{name}");
    Some(vec![
        cmd(&["chown", &format!("{name}:{name}"), &home], None),
        cmd(&["chmod", "0700", &home], None),
    ])
}

/// At login, with the password just checked: load the key if needed, then mount. Nothing for a
/// plain home or one already open (another session).
pub fn unlock(name: &str, state: Option<State>, password: &str) -> Vec<Cmd> {
    let (Some(ds), Some(s)) = (dataset(name), state) else {
        return vec![];
    };
    if !s.encrypted {
        return vec![];
    }
    let mut v = vec![];
    if !s.key_loaded {
        v.push(cmd(&["zfs", "load-key", &ds], Some(password)));
    }
    if !s.mounted {
        v.push(cmd(&["zfs", "mount", &ds], None));
    }
    v
}

/// At logout: unmount (refused while something still has a file open: then it stays open), then
/// forget the key.
pub fn lock(name: &str, state: Option<State>) -> Vec<Cmd> {
    let (Some(ds), Some(s)) = (dataset(name), state) else {
        return vec![];
    };
    if !s.encrypted {
        return vec![];
    }
    let mut v = vec![];
    if s.mounted {
        v.push(cmd(&["zfs", "unmount", &ds], None));
    }
    if s.key_loaded {
        v.push(cmd(&["zfs", "unload-key", &ds], None));
    }
    v
}

/// New password, new key (the key must be loaded: the account is logged in). Run together with
/// the password change itself, or the next login cannot open the files.
pub fn change_key(name: &str, new_password: &str) -> Option<Cmd> {
    let ds = dataset(name)?;
    password_ok(new_password).then(|| {
        cmd(
            &[
                "zfs",
                "change-key",
                "-o",
                "keyformat=passphrase",
                "-o",
                "keylocation=prompt",
                &ds,
            ],
            Some(new_password),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(v: &[Cmd]) -> String {
        v.iter()
            .map(|c| {
                let pw = if c.stdin.is_some() { " <pw" } else { "" };
                format!("{}{pw}", c.argv.join(" "))
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    #[test]
    fn creates_an_encrypted_home_before_the_account() {
        let v = create("ann", "correct horse").unwrap();
        assert_eq!(
            shown(&v),
            "zfs create -o encryption=aes-256-gcm -o keyformat=passphrase -o keylocation=prompt \
             -o canmount=noauto -o mountpoint=/home/ann zroot/home/ann <pw; zfs mount zroot/home/ann"
        );
        assert_eq!(v[0].stdin.as_deref(), Some("correct horse\n"));
        assert!(
            v.iter()
                .all(|c| !c.argv.iter().any(|a| a.contains("horse"))),
            "never an argument"
        );
        assert_eq!(create("ann", "short"), None, "ZFS needs 8 bytes");
        assert_eq!(create("ann", "two\nlines"), None);
        assert_eq!(create("../root", "correct horse"), None);
        assert_eq!(
            shown(&after_add("ann").unwrap()),
            "chown ann:ann /home/ann; chmod 0700 /home/ann"
        );
    }

    #[test]
    fn status_from_zfs_get() {
        assert_eq!(
            status_argv("ann").unwrap().join(" "),
            "zfs get -H -o value encryption,keystatus,mounted zroot/home/ann"
        );
        let s = |t| parse_status(Some(t));
        assert_eq!(parse_status(None), None, "no dataset");
        assert_eq!(s("off\n-\nyes\n").map(|s| s.encrypted), Some(false));
        assert_eq!(
            s("aes-256-gcm\nunavailable\nno\n"),
            Some(State {
                encrypted: true,
                key_loaded: false,
                mounted: false
            })
        );
        assert_eq!(s("garbage\n"), None);
    }

    #[test]
    fn login_unlocks_and_logout_locks() {
        let locked = Some(State {
            encrypted: true,
            key_loaded: false,
            mounted: false,
        });
        let open = Some(State {
            encrypted: true,
            key_loaded: true,
            mounted: true,
        });
        let plain = Some(State {
            encrypted: false,
            key_loaded: false,
            mounted: true,
        });
        assert_eq!(
            shown(&unlock("ann", locked, "correct horse")),
            "zfs load-key zroot/home/ann <pw; zfs mount zroot/home/ann"
        );
        assert!(
            unlock("ann", open, "x").is_empty(),
            "another session has it open"
        );
        assert!(unlock("ann", plain, "x").is_empty());
        assert!(unlock("ann", None, "x").is_empty(), "plain folder");
        assert_eq!(
            shown(&lock("ann", open)),
            "zfs unmount zroot/home/ann; zfs unload-key zroot/home/ann"
        );
        assert!(lock("ann", locked).is_empty());
        assert!(lock("ann", plain).is_empty());
    }

    #[test]
    fn password_change_rekeys() {
        let c = change_key("ann", "new password").unwrap();
        assert_eq!(
            c.argv.join(" "),
            "zfs change-key -o keyformat=passphrase -o keylocation=prompt zroot/home/ann"
        );
        assert_eq!(c.stdin.as_deref(), Some("new password\n"));
        assert_eq!(change_key("ann", "short"), None);
    }
}
