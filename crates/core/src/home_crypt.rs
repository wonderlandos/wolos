//! "Encrypt my files": the account's home is a systemd-homed LUKS home (`homectl`), encrypted
//! under the login password. At boot it stays locked; the login window activates it with the
//! password it has just checked, and logging out deactivates it again when nothing still uses it.
//! Off by default until it is proven on hardware (archicture/modules/security.md).
//!
//! Pure: commands for the caller to run (wol-login, the Setup Assistant plan). Passwords travel
//! on stdin or in the environment homectl reads (`PASSWORD`, `NEWPASSWORD`), never as arguments.

/// Password limits (account passwords are 8..=128 characters).
pub const MIN_PASSWORD: usize = 8;
pub const MAX_PASSWORD: usize = 512;

/// One command; `stdin` and `env` carry secrets (never an argument: `ps` shows arguments).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    pub argv: Vec<String>,
    pub stdin: Option<String>,
    pub env: Vec<(String, String)>,
}

fn cmd(argv: &[&str]) -> Cmd {
    Cmd {
        argv: argv.iter().map(|s| (*s).to_owned()).collect(),
        stdin: None,
        env: vec![],
    }
}

fn with_env(mut c: Cmd, k: &str, v: &str) -> Cmd {
    c.env.push((k.into(), v.into()));
    c
}

/// The account name; None for a name WolOS would not create.
pub fn home(name: &str) -> Option<&str> {
    crate::settings::users::is_valid_name(name).then_some(name)
}

/// A password homed accepts (no newline).
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

/// `homectl inspect --json=short`; fails (None) when the account is not a homed home.
pub fn status_argv(name: &str) -> Option<Vec<String>> {
    let n = home(name)?;
    Some(
        ["homectl", "inspect", "--json=short", n]
            .map(String::from)
            .to_vec(),
    )
}

/// None: not a homed home (a plain folder, nothing to unlock).
pub fn parse_status(out: Option<&str>) -> Option<State> {
    let v: serde_json::Value = serde_json::from_str(out?).ok()?;
    let storage = v["storage"].as_str()?;
    let open = matches!(
        v["status"]["state"].as_str().or(v["state"].as_str()),
        Some("active" | "activating" | "active-for-login")
    );
    Some(State {
        encrypted: storage == "luks",
        key_loaded: open,
        mounted: open,
    })
}

/// Instead of `useradd`: homed creates the account and its LUKS home in one step; the user
/// record (with the password in its secret section) goes on stdin.
pub fn create(name: &str, password: &str) -> Option<Vec<Cmd>> {
    let n = home(name)?;
    if !password_ok(password) {
        return None;
    }
    let record = serde_json::json!({
        "userName": n,
        "storage": "luks",
        "secret": {"password": [password]},
    });
    let mut c = cmd(&["homectl", "create", "--identity=-"]);
    c.stdin = Some(format!("{record}\n"));
    Some(vec![c])
}

/// After create: homed already owns the folder and makes it private.
pub fn after_add(name: &str) -> Option<Vec<Cmd>> {
    home(name).map(|_| vec![])
}

/// At login, with the password just checked: activate the home. Nothing for a plain home or one
/// already open (another session).
pub fn unlock(name: &str, state: Option<State>, password: &str) -> Vec<Cmd> {
    let (Some(n), Some(s)) = (home(name), state) else {
        return vec![];
    };
    if !s.encrypted || s.mounted {
        return vec![];
    }
    vec![with_env(
        cmd(&["homectl", "activate", n]),
        "PASSWORD",
        password,
    )]
}

/// At logout: deactivate (refused while something still has a file open: then it stays open).
pub fn lock(name: &str, state: Option<State>) -> Vec<Cmd> {
    let (Some(n), Some(s)) = (home(name), state) else {
        return vec![];
    };
    if !s.encrypted || !s.mounted {
        return vec![];
    }
    vec![cmd(&["homectl", "deactivate", n])]
}

/// New password, new key (the home must be active: the account is logged in). Run together with
/// the password change itself, or the next login cannot open the files.
pub fn change_key(name: &str, new_password: &str) -> Option<Cmd> {
    let n = home(name)?;
    password_ok(new_password)
        .then(|| with_env(cmd(&["homectl", "passwd", n]), "NEWPASSWORD", new_password))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(v: &[Cmd]) -> String {
        v.iter()
            .map(|c| c.argv.join(" "))
            .collect::<Vec<_>>()
            .join("; ")
    }

    #[test]
    fn creates_the_account_with_an_encrypted_home() {
        let v = create("ann", "correct horse").unwrap();
        assert_eq!(shown(&v), "homectl create --identity=-");
        let rec: serde_json::Value = serde_json::from_str(v[0].stdin.as_deref().unwrap()).unwrap();
        assert_eq!(rec["userName"], "ann");
        assert_eq!(rec["storage"], "luks");
        assert_eq!(rec["secret"]["password"][0], "correct horse");
        assert!(v[0].argv.iter().all(|a| !a.contains("horse")));
        assert_eq!(create("ann", "short"), None);
        assert_eq!(create("ann", "two\nlines"), None);
        assert_eq!(create("../root", "correct horse"), None);
        assert_eq!(after_add("ann"), Some(vec![]));
    }

    #[test]
    fn status_from_homectl() {
        let s = |t: &str| parse_status(Some(t));
        assert_eq!(parse_status(None), None, "not a homed home");
        assert_eq!(
            s(r#"{"userName":"ann","storage":"luks","status":{"state":"inactive"}}"#),
            Some(State {
                encrypted: true,
                key_loaded: false,
                mounted: false
            })
        );
        assert_eq!(
            s(r#"{"storage":"luks","status":{"state":"active"}}"#).map(|s| s.mounted),
            Some(true)
        );
        assert_eq!(
            s(r#"{"storage":"directory","status":{"state":"active"}}"#).map(|s| s.encrypted),
            Some(false)
        );
        assert_eq!(s("garbage"), None);
        assert_eq!(
            status_argv("ann").unwrap().join(" "),
            "homectl inspect --json=short ann"
        );
        assert_eq!(status_argv("-x"), None);
    }

    #[test]
    fn login_unlocks_and_logout_locks() {
        let st = |encrypted, open| {
            Some(State {
                encrypted,
                key_loaded: open,
                mounted: open,
            })
        };
        let u = unlock("ann", st(true, false), "correct horse");
        assert_eq!(shown(&u), "homectl activate ann");
        assert_eq!(u[0].env, [("PASSWORD".into(), "correct horse".into())]);
        assert!(
            unlock("ann", st(true, true), "x").is_empty(),
            "already open"
        );
        assert!(unlock("ann", st(false, true), "x").is_empty());
        assert!(unlock("ann", None, "x").is_empty(), "plain folder");
        assert_eq!(
            shown(&lock("ann", st(true, true))),
            "homectl deactivate ann"
        );
        assert!(lock("ann", st(true, false)).is_empty());
        assert!(lock("ann", st(false, true)).is_empty());
    }

    #[test]
    fn password_change_rekeys() {
        let c = change_key("ann", "new password").unwrap();
        assert_eq!(c.argv.join(" "), "homectl passwd ann");
        assert_eq!(c.env, [("NEWPASSWORD".into(), "new password".into())]);
        assert_eq!(change_key("ann", "short"), None);
    }
}
