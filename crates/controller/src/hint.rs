// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol hint <command line> [status] [--not-found]` (ADR 0032 "Console"): one plain sentence and
//! the WolOS way to do it, for a command that is missing (`apt`, `dnf`, `brew` ...) or that failed
//! in a way WolOS knows (pacman without sudo, a write under the read-only `/usr`, `su` with no root
//! password). The rows live in one table, [`TABLE`] (`console/hints.tsv`, format in its header);
//! every WolOS shell reads its first column once and calls this only for the commands it names.
//! Exit 0 when a hint was printed (stderr; `--json`: one object on stdout), 1 when none matched.
use crate::cli::Invocation;
use crate::run::{Env, emit_json};
use std::path::Path;

/// The installed table; the shells read the same file.
pub const TABLE: &str = "/usr/share/wolos/console/hints.tsv";
/// The table built in, for a system without the file (and the tests).
pub const BUILTIN: &str =
    include_str!("../../../desktop/overlay/usr/share/wolos/console/hints.tsv");
/// Services WolOS sets up (`managed`).
pub const PRESET: &str = "/usr/lib/systemd/system-preset/50-wolos.preset";
/// Condition words a row may use (test-checked against the table).
pub const WORDS: &[&str] = &[
    "missing", "failed", "any", "user", "root", "sealed", "nowheel", "managed",
];
/// Condition words that take `=PATTERN` (`status=N` takes a number).
pub const KEYED: &[&str] = &["arg", "last", "typed", "status"];
/// Placeholders a sentence or suggestion may use.
pub const PLACEHOLDERS: &[&str] = &["{cmd}", "{args}", "{targets}", "{names}", "{first}"];

/// One row of the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub command: String,
    pub condition: String,
    pub sentence: String,
    pub suggestion: String,
}

/// Rows of a table text: `#` lines and blank lines skipped, a line without four columns ignored.
pub fn rows(text: &str) -> Vec<Row> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() == 4).then(|| Row {
                command: f[0].trim().into(),
                condition: f[1].trim().into(),
                sentence: f[2].trim().into(),
                suggestion: f[3].trim().into(),
            })
        })
        .collect()
}

/// What happened to the command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not installed (the shell's command-not-found).
    Missing,
    /// Ran and exited non-zero (`None`: status not given).
    Failed(Option<i32>),
}

/// The command line split into words (quotes removed; no expansion), leading `VAR=value` skipped.
pub fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quote, mut any) = (None::<char>, false);
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            (None, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    let skip = out
        .iter()
        .take_while(|w| {
            w.split_once('=').is_some_and(|(k, _)| {
                !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        })
        .count();
    out.drain(..skip);
    out
}

/// `*` is any text; `a|b` either.
pub fn pattern(pat: &str, s: &str) -> bool {
    pat.split('|').any(|p| glob(p.as_bytes(), s.as_bytes()))
}

fn glob(p: &[u8], s: &[u8]) -> bool {
    match p.split_first() {
        None => s.is_empty(),
        Some((b'*', rest)) => (0..=s.len()).any(|i| glob(rest, &s[i..])),
        Some((c, rest)) => s.first() == Some(c) && glob(rest, &s[1..]),
    }
}

/// Facts the conditions ask about, each read once and only when a row needs it.
struct Facts<'a> {
    env: &'a mut dyn Env,
    sealed: Option<bool>,
    wheel: Option<bool>,
    preset: Option<Vec<String>>,
}

impl Facts<'_> {
    fn sealed(&mut self) -> bool {
        if self.sealed.is_none() {
            let cmdline = self.env.read_file(Path::new("/proc/cmdline"));
            let ro = self.env.capture(&wol_core::update::pkg::RO_ARGV);
            self.sealed = Some(wol_core::update::pkg::is_sealed(
                cmdline.as_deref(),
                ro.as_deref(),
            ));
        }
        self.sealed == Some(true)
    }

    /// In `wheel`: listed in `/etc/group`, or it is the account's own group in `/etc/passwd`.
    fn wheel(&mut self) -> bool {
        if self.wheel.is_none() {
            let user = self
                .env
                .var("USER")
                .or_else(|| self.env.var("LOGNAME"))
                .unwrap_or_default();
            let group = self
                .env
                .read_file(Path::new("/etc/group"))
                .unwrap_or_default();
            let passwd = self
                .env
                .read_file(Path::new("/etc/passwd"))
                .unwrap_or_default();
            let wheel = group.lines().find_map(|l| {
                let f: Vec<&str> = l.split(':').collect();
                (f.len() >= 4 && f[0] == "wheel").then(|| (f[2].to_string(), f[3].to_string()))
            });
            self.wheel = Some(wheel.is_some_and(|(gid, members)| {
                members.split(',').any(|m| m.trim() == user)
                    || passwd.lines().any(|l| {
                        let f: Vec<&str> = l.split(':').collect();
                        f.len() >= 4 && f[0] == user && f[3] == gid
                    })
            }));
        }
        self.wheel == Some(true)
    }

    fn managed(&mut self, args: &[String]) -> bool {
        if self.preset.is_none() {
            let text = self.env.read_file(Path::new(PRESET)).unwrap_or_default();
            self.preset = Some(
                text.lines()
                    .filter_map(|l| {
                        let mut w = l.split_whitespace();
                        matches!(w.next(), Some("enable" | "disable"))
                            .then(|| w.next().map(str::to_string))
                            .flatten()
                    })
                    .collect(),
            );
        }
        let units = self.preset.as_deref().unwrap_or_default();
        args.iter().filter(|a| !a.starts_with('-')).any(|a| {
            let unit = if a.contains('.') {
                a.clone()
            } else {
                format!("{a}.service")
            };
            units.contains(&unit)
        })
    }
}

/// On `$PATH` (an `X` after `sudo` counts as missing when it is not).
fn on_path(env: &dyn Env, name: &str) -> bool {
    if name.contains('/') {
        return Path::new(name).exists();
    }
    env.var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .any(|d| Path::new(d).join(name).exists())
}

fn holds(word: &str, typed: &str, args: &[String], out: Outcome, f: &mut Facts) -> bool {
    let (key, val) = word.split_once('=').unwrap_or((word, ""));
    match key {
        "missing" => out == Outcome::Missing,
        "failed" => matches!(out, Outcome::Failed(_)),
        "any" => true,
        "user" => !f.env.is_root(),
        "root" => f.env.is_root(),
        "sealed" => f.sealed(),
        "nowheel" => !f.env.is_root() && !f.wheel(),
        "managed" => f.managed(args),
        "status" => val
            .parse()
            .ok()
            .is_some_and(|n| out == Outcome::Failed(Some(n))),
        "arg" => args.iter().any(|a| pattern(val, a)),
        "last" => args.last().is_some_and(|a| pattern(val, a)),
        "typed" => pattern(val, typed),
        // A word this build does not know never matches (a newer table on an older `wol`).
        _ => false,
    }
}

fn row_matches(row: &Row, typed: &str, args: &[String], out: Outcome, f: &mut Facts) -> bool {
    row.condition
        .split_whitespace()
        .all(|w| match w.strip_prefix('!') {
            Some(w) => !holds(w, typed, args, out, f),
            None => holds(w, typed, args, out, f),
        })
}

/// `sudo`'s options that take a value (`sudo -u bob apt ...`).
const SUDO_VALUE: &[&str] = &[
    "-u", "-g", "-C", "-D", "-h", "-p", "-r", "-t", "-U", "-R", "-T",
];

/// A matched row with the words it matched on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub row: Row,
    pub command: String,
    pub args: Vec<String>,
}

/// The first row for this command line, then (for `sudo X`) for X.
fn find(table: &[Row], line: &str, out: Outcome, f: &mut Facts) -> Option<Found> {
    let mut w = words(line);
    let mut out = out;
    loop {
        if w.is_empty() {
            return None;
        }
        let typed = w[0].clone();
        let name = typed.rsplit('/').next().unwrap_or_default().to_string();
        let args = w[1..].to_vec();
        if let Some(row) = table
            .iter()
            .filter(|r| r.command == name)
            .find(|r| row_matches(r, &typed, &args, out, f))
        {
            return Some(Found {
                row: row.clone(),
                command: name,
                args,
            });
        }
        if name != "sudo" || out == Outcome::Missing {
            return None;
        }
        // `sudo X ...` failed: look X up; sudo reports a missing X with its own status.
        let mut i = 1;
        while i < w.len() && w[i].starts_with('-') {
            i += if SUDO_VALUE.contains(&w[i].as_str()) {
                2
            } else {
                1
            };
            if w.get(i - 1).is_some_and(|x| x == "--") {
                break;
            }
        }
        w.drain(..i.min(w.len()));
        if let Some(x) = w.first()
            && !on_path(f.env, x)
        {
            out = Outcome::Missing;
        }
    }
}

/// The row's text with the placeholders filled in.
pub fn fill(text: &str, command: &str, args: &[String]) -> String {
    let targets: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-'))
        .collect();
    let or_name = |v: &[&str]| {
        if v.is_empty() {
            "name".to_string()
        } else {
            v.join(" ")
        }
    };
    text.replace("{cmd}", command)
        .replace("{args}", &args.join(" "))
        .replace("{targets}", &or_name(&targets))
        .replace("{names}", &or_name(targets.get(1..).unwrap_or_default()))
        .replace("{first}", &or_name(&targets[..targets.len().min(1)]))
}

/// The person's language for messages: `ll_CC`, then `ll` (none for C, POSIX and English).
pub fn languages(env: &dyn Env) -> Vec<String> {
    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| env.var(k).filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let lang = lang.split(['.', '@']).next().unwrap_or_default();
    if lang.is_empty() || lang == "C" || lang == "POSIX" || lang.starts_with("en") {
        return vec![];
    }
    let mut out = vec![lang.to_string()];
    if let Some((ll, _)) = lang.split_once('_') {
        out.push(ll.to_string());
    }
    out
}

/// The table, and a translated row for the same command and condition when there is one.
fn translated(env: &dyn Env, row: &Row) -> Row {
    let dir = Path::new(TABLE).parent().unwrap_or(Path::new("/"));
    for l in languages(env) {
        let Some(text) = env.read_file(&dir.join(format!("hints.{l}.tsv"))) else {
            continue;
        };
        if let Some(t) = rows(&text)
            .into_iter()
            .find(|t| t.command == row.command && t.condition == row.condition)
        {
            return t;
        }
    }
    row.clone()
}

/// The hint for one command line, if any (the table from [`TABLE`], else [`BUILTIN`]).
pub fn lookup(env: &mut dyn Env, line: &str, out: Outcome) -> Option<(String, Option<String>)> {
    let text = env
        .read_file(Path::new(TABLE))
        .unwrap_or_else(|| BUILTIN.to_string());
    let table = rows(&text);
    let found = {
        let mut f = Facts {
            env: &mut *env,
            sealed: None,
            wheel: None,
            preset: None,
        };
        find(&table, line, out, &mut f)?
    };
    let row = translated(env, &found.row);
    let sentence = fill(&row.sentence, &found.command, &found.args);
    let suggestion =
        (row.suggestion != "-").then(|| fill(&row.suggestion, &found.command, &found.args));
    Some((sentence, suggestion))
}

/// `wol hint`.
pub fn run(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let line = inv.get("command").unwrap_or_default().to_string();
    let out = if inv.flag("not-found") {
        Outcome::Missing
    } else {
        match inv.get("status").map(str::parse::<i32>) {
            Some(Ok(0)) => return 1,
            Some(Ok(n)) => Outcome::Failed(Some(n)),
            _ => Outcome::Failed(None),
        }
    };
    let Some((sentence, suggestion)) = lookup(env, &line, out) else {
        return 1;
    };
    if inv.flag("json") {
        emit_json(
            env,
            &serde_json::json!({"sentence": sentence, "suggestion": suggestion}),
        );
    } else {
        let mut s = format!("{sentence}\n");
        if let Some(x) = suggestion {
            s.push_str(&format!("  {x}\n"));
        }
        env.err(&s);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::FakeEnv;
    use std::process::Command;

    fn hint(env: &mut FakeEnv, line: &str, out: Outcome) -> Option<(String, Option<String>)> {
        lookup(env, line, out)
    }
    fn suggestion(env: &mut FakeEnv, line: &str, out: Outcome) -> Option<String> {
        hint(env, line, out).and_then(|h| h.1)
    }
    const FAILED: Outcome = Outcome::Failed(Some(1));

    fn sealed_env(root: bool) -> FakeEnv {
        FakeEnv {
            root,
            files: vec![(
                "/proc/cmdline".into(),
                "root=UUID=x rootflags=subvol=@wolos-a ro".into(),
            )],
            captured: Some("ro=true\n".into()),
            ..Default::default()
        }
    }

    #[test]
    fn the_table_is_well_formed_and_plain() {
        let mut lines = 0;
        for l in BUILTIN
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            lines += 1;
            assert_eq!(l.split('\t').count(), 4, "four columns: {l}");
        }
        let table = rows(BUILTIN);
        assert_eq!(table.len(), lines);
        for r in &table {
            assert!(
                !r.command.is_empty() && !r.command.contains(['|', ' ', '/']),
                "{r:?}"
            );
            for w in r.condition.split_whitespace() {
                let w = w.strip_prefix('!').unwrap_or(w);
                let known = match w.split_once('=') {
                    Some((k, v)) => KEYED.contains(&k) && !v.is_empty(),
                    None => WORDS.contains(&w),
                };
                assert!(known, "unknown condition word {w} in {r:?}");
            }
            for text in [&r.sentence, &r.suggestion] {
                let mut rest = text.as_str();
                while let Some(i) = rest.find('{') {
                    let end = rest[i..].find('}').map_or(rest.len(), |e| i + e + 1);
                    assert!(PLACEHOLDERS.contains(&&rest[i..end]), "{text}");
                    rest = &rest[end..];
                }
            }
            // UX-LESSONS: a sentence for a person: no blame, no jargon, never "safe".
            let low = r.sentence.to_lowercase();
            for bad in [
                "safe", "sealed", "erofs", "illegal", "invalid", "wrong", "you must", "error",
                "subvol", "btrfs", "sysext", "polkit", "fhs",
            ] {
                assert!(!low.contains(bad), "{bad} in {r:?}");
            }
            assert!(r.sentence.ends_with('.'), "{r:?}");
            assert!(!r.suggestion.is_empty(), "{r:?}");
        }
        // The commands the console promises a hint for.
        for c in [
            "apt",
            "apt-get",
            "dpkg",
            "dnf",
            "yum",
            "zypper",
            "snap",
            "brew",
            "port",
            "emerge",
            "nix",
            "nix-env",
            "flatpak",
            "su",
            "pacman",
            "sudo",
            "systemctl",
            "cp",
            "mv",
            "rm",
            "mkdir",
            "touch",
        ] {
            assert!(table.iter().any(|r| r.command == c), "no row for {c}");
        }
    }

    #[test]
    fn suggestions_name_real_wol_commands() {
        for r in rows(BUILTIN) {
            let s = fill(&r.suggestion, &r.command, &["x".into(), "y".into()]);
            if let Some(rest) = s.strip_prefix("wol ") {
                let argv: Vec<&str> = rest.split_whitespace().collect();
                assert!(crate::cli::parse(&argv).is_ok(), "{s}");
            }
        }
    }

    /// QA-PWR-9: a module's tool typed before the module is installed names the module (git,
    /// docker and fish got a bare 'command not found').
    #[test]
    fn module_tools_name_their_module() {
        let mut env = FakeEnv::default();
        let m = Outcome::Missing;
        for (line, want) in [
            ("git status", "wol modules install dev"),
            ("docker run hello-world", "wol modules install dev"),
            ("podman ps", "wol modules install dev"),
            ("fish", "wol modules install console"),
            ("zellij", "wol modules install console"),
        ] {
            assert_eq!(
                suggestion(&mut env, line, m).as_deref(),
                Some(want),
                "{line}"
            );
        }
        let (s, _) = hint(&mut env, "git status", m).unwrap();
        assert!(s.starts_with("git comes with the Developer module"), "{s}");
        // Installed and failing: no hint (git's own message says why).
        assert!(hint(&mut env, "git status", FAILED).is_none());
    }

    #[test]
    fn package_tools_from_other_systems() {
        let mut env = FakeEnv::default();
        let m = Outcome::Missing;
        assert_eq!(
            suggestion(&mut env, "apt install firefox gimp", m).as_deref(),
            Some("wopkg install firefox gimp")
        );
        assert_eq!(
            suggestion(&mut env, "apt-get remove vlc", m).as_deref(),
            Some("wopkg remove vlc")
        );
        assert_eq!(
            suggestion(&mut env, "dnf search editor", m).as_deref(),
            Some("wopkg search editor")
        );
        assert_eq!(
            suggestion(&mut env, "zypper dup", m).as_deref(),
            Some("wol update install")
        );
        assert_eq!(
            suggestion(&mut env, "brew install", m).as_deref(),
            Some("wopkg install name")
        );
        assert_eq!(
            suggestion(&mut env, "emerge firefox", m).as_deref(),
            Some("wopkg install firefox")
        );
        assert_eq!(
            suggestion(&mut env, "flatpak install flathub org.gimp.GIMP", m).as_deref(),
            Some("sudo pacman -S flatpak")
        );
        let (s, _) = hint(&mut env, "apt", m).unwrap();
        assert!(
            s.starts_with("apt is the package tool of Debian and Ubuntu"),
            "{s}"
        );
        // A tool the person installed and that failed on its own is theirs.
        assert!(hint(&mut env, "brew install x", FAILED).is_none());
        assert!(hint(&mut env, "ls /nowhere", FAILED).is_none());
        assert!(hint(&mut env, "", m).is_none());
    }

    #[test]
    fn sudo_looks_up_the_command_behind_it() {
        let mut env = FakeEnv {
            vars: vec![
                ("PATH".into(), "/nonexistent-wolos-hint".into()),
                ("USER".into(), "ana".into()),
            ],
            files: vec![("/etc/group".into(), "wheel:x:998:ana\n".into())],
            ..Default::default()
        };
        assert_eq!(
            suggestion(&mut env, "sudo -E apt install vlc", FAILED).as_deref(),
            Some("wopkg install vlc")
        );
        assert_eq!(
            suggestion(&mut env, "sudo -u bob apt-get upgrade", FAILED).as_deref(),
            Some("wol update install")
        );
        // Not an administrator: that comes first.
        let mut env = FakeEnv {
            vars: vec![("USER".into(), "kid".into())],
            files: vec![("/etc/group".into(), "wheel:x:998:ana\n".into())],
            ..Default::default()
        };
        let (s, sug) = hint(&mut env, "sudo pacman -Syu", FAILED).unwrap();
        assert!(s.contains("not an administrator"), "{s}");
        assert_eq!(sug, None);
        // An administrator whose sudo command failed for its own reasons: no hint.
        let mut env = FakeEnv {
            vars: vec![
                ("USER".into(), "ana".into()),
                ("PATH".into(), "/usr/bin:/bin".into()),
            ],
            files: vec![("/etc/group".into(), "wheel:x:998:ana\n".into())],
            ..Default::default()
        };
        assert!(hint(&mut env, "sudo ls /nowhere", FAILED).is_none());
        // wheel as the account's own group counts too.
        let mut env = FakeEnv {
            vars: vec![("USER".into(), "ana".into())],
            files: vec![
                ("/etc/group".into(), "wheel:x:998:\n".into()),
                (
                    "/etc/passwd".into(),
                    "ana:x:1000:998::/home/ana:/bin/zsh\n".into(),
                ),
            ],
            ..Default::default()
        };
        assert!(hint(&mut env, "sudo true", FAILED).is_none());
    }

    #[test]
    fn pacman_without_sudo_and_on_the_read_only_system() {
        let mut env = FakeEnv::default();
        assert_eq!(
            suggestion(&mut env, "pacman -Syu", FAILED).as_deref(),
            Some("wol update install")
        );
        assert_eq!(
            suggestion(&mut env, "pacman -S --needed gimp", FAILED).as_deref(),
            Some("sudo pacman -S --needed gimp")
        );
        // Searches and queries fail when nothing matches: no hint.
        assert!(hint(&mut env, "pacman -Ss nothing", FAILED).is_none());
        assert!(hint(&mut env, "pacman -Si nothing", FAILED).is_none());
        assert!(hint(&mut env, "pacman -Qi nothing", FAILED).is_none());
        // Root with the real pacman on a sealed slot: the WolOS pacman instead.
        let mut env = sealed_env(true);
        assert_eq!(
            suggestion(&mut env, "/usr/bin/pacman -S gimp", FAILED).as_deref(),
            Some("wol pkg -- -S gimp")
        );
        assert_eq!(
            suggestion(&mut env, "/usr/bin/pacman -Syu", FAILED).as_deref(),
            Some("wol update install")
        );
        // The WolOS pacman says what went wrong itself.
        assert!(hint(&mut env, "pacman -S gimp", FAILED).is_none());
        // Developer mode: writable, no hint.
        let mut env = FakeEnv {
            root: true,
            ..sealed_env(true)
        };
        env.captured = Some("ro=false\n".into());
        assert!(hint(&mut env, "/usr/bin/pacman -S gimp", FAILED).is_none());
    }

    #[test]
    fn writes_under_usr_and_managed_services() {
        let mut env = sealed_env(false);
        let (s, sug) = hint(&mut env, "cp tool /usr/bin/", FAILED).unwrap();
        assert!(s.contains("~/.local/bin"), "{s}");
        assert_eq!(sug, None);
        assert!(hint(&mut env, "rm -f /usr/bin/x", FAILED).is_some());
        assert!(hint(&mut env, "sed -i s/a/b/ /usr/share/x", FAILED).is_some());
        // Reading from /usr is not writing to it.
        assert!(hint(&mut env, "cp /usr/share/x /root/", FAILED).is_none());
        assert!(hint(&mut env, "sed s/a/b/ /usr/share/x", FAILED).is_none());
        assert!(hint(&mut FakeEnv::default(), "cp a /usr/bin/", FAILED).is_none());

        let mut env = FakeEnv {
            files: vec![(
                PRESET.into(),
                "enable wolos-hw.service\nenable iwd.service\n".into(),
            )],
            ..Default::default()
        };
        assert_eq!(
            suggestion(&mut env, "systemctl disable --now wolos-hw", FAILED).as_deref(),
            Some("wol doctor")
        );
        assert_eq!(
            suggestion(&mut env, "systemctl enable sshd", FAILED).as_deref(),
            Some("sudo systemctl enable sshd")
        );
        assert!(hint(&mut env, "systemctl status sshd", FAILED).is_none());
        // QA-PWR-5: `sudo systemctl --user` talks to root's user manager, which is not there.
        assert!(hint(&mut env, "systemctl --user enable foo.timer", FAILED).is_none());
    }

    #[test]
    fn su_and_the_status_argument() {
        let mut env = FakeEnv::default();
        assert_eq!(
            suggestion(&mut env, "su", Outcome::Failed(None)).as_deref(),
            Some("sudo -i")
        );
        let inv = crate::cli::parse(&["hint", "su", "0"]).unwrap();
        assert_eq!(run(&mut env, &inv), 1, "a command that worked gets no hint");
        let inv = crate::cli::parse(&["hint", "su -s /bin/sh", "1"]).unwrap();
        assert_eq!(run(&mut env, &inv), 0);
        assert!(env.err.contains("sudo passwd root") && env.err.contains("  sudo -i\n"));
        let mut env = FakeEnv::default();
        let inv = crate::cli::parse(&["hint", "apt install vlc", "--not-found", "--json"]).unwrap();
        assert_eq!(run(&mut env, &inv), 0);
        let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
        assert_eq!(v["suggestion"], "wopkg install vlc");
        let inv = crate::cli::parse(&["hint", "frobnicate", "--not-found"]).unwrap();
        assert_eq!(run(&mut env, &inv), 1);
    }

    #[test]
    fn words_patterns_and_languages() {
        assert_eq!(words("A=1 sudo 'x y' \"\" z"), ["sudo", "x y", "", "z"]);
        assert!(pattern("-S*u*", "-Syu") && pattern("-S*u*", "-Suy"));
        assert!(!pattern("-S*u*", "-Ss") && pattern("a|b", "b") && !pattern("a", "ab"));
        let tr = "apt\tmissing\tapt vem do Debian e do Ubuntu; no WolOS, o wopkg instala programas.\twopkg help\n";
        let mut env = FakeEnv {
            vars: vec![
                ("LANG".into(), "pt_BR.UTF-8".into()),
                ("LC_MESSAGES".into(), String::new()),
            ],
            files: vec![("/usr/share/wolos/console/hints.pt.tsv".into(), tr.into())],
            ..Default::default()
        };
        let (s, _) = hint(&mut env, "apt", Outcome::Missing).unwrap();
        assert!(s.starts_with("apt vem do Debian"), "{s}");
        // A row the translation lacks stays in English.
        let (s, _) = hint(&mut env, "apt install x", Outcome::Missing).unwrap();
        assert!(s.contains("install programs"), "{s}");
        let c = FakeEnv {
            vars: vec![("LC_ALL".into(), "C.UTF-8".into())],
            ..Default::default()
        };
        assert!(languages(&c).is_empty());
    }

    // The shells: the setup files wire the table, `su`, command-not-found and the post-command hook.
    fn share(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../desktop/overlay/usr/share/wolos")
            .join(rel)
    }

    #[test]
    fn every_shell_wires_hints_and_su() {
        for (rel, wants) in [
            (
                "zsh/zshrc",
                &["command_not_found_handler", "precmd_functions", "su()"][..],
            ),
            (
                "bash/bashrc",
                &["command_not_found_handle", "PROMPT_COMMAND", "su()"],
            ),
            (
                "fish/config.fish",
                &["fish_command_not_found", "fish_postexec", "function su"],
            ),
            (
                "nu/autoload/wolos.nu",
                &["command_not_found", "pre_prompt", "def --wrapped su"],
            ),
            (
                "pwsh/profile.ps1",
                &[
                    "CommandNotFoundAction",
                    "Invoke-Starship-PreCommand",
                    "function global:su",
                ],
            ),
        ] {
            let text = std::fs::read_to_string(share(rel)).unwrap();
            for w in wants
                .iter()
                .chain(&["console/hints.tsv", "-iu", "sh -c", "hint"])
            {
                assert!(text.contains(w), "{rel} lacks {w}");
            }
            // wol-login starts sessions through `su -l`: never an alias or a PATH wrapper.
            assert!(!text.contains("alias su"), "{rel}");
        }
    }

    /// A temp dir with a fake `wol` and `sudo` that log their arguments, one line per call.
    fn fakes(name: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("wol-hint-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("bin")).unwrap();
        for tool in ["wol", "sudo"] {
            let p = d.join("bin").join(tool);
            let body = format!(
                "#!/bin/sh\nprintf '%s|' {tool} \"$@\" >> \"$LOG\"\necho >> \"$LOG\"\n\
                 case \"$1\" in hint) case \"$2\" in dnf*|cp*) exit 0;; esac; exit 1;; esac\n"
            );
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        d
    }

    fn run_shell(shell: &str, args: &[&str], d: &Path, script: &str) -> (String, String) {
        let path = format!("{}:/usr/bin:/bin", d.join("bin").display());
        let out = Command::new(shell)
            .args(args)
            .env_clear()
            .env("HOME", d)
            .env("TERM", "dumb")
            .env("PATH", &path)
            .env("LOG", d.join("log"))
            .env("WOLOS_PROMPT", "pure")
            .env("WOLOS_HINTS", share("console/hints.tsv"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap().write_all(script.as_bytes())?;
                c.wait_with_output()
            })
            .unwrap();
        let log = std::fs::read_to_string(d.join("log")).unwrap_or_default();
        (log, String::from_utf8_lossy(&out.stderr).into_owned())
    }

    const SCRIPT: &str = "su\nsu -\nsu bob\nsu -l bob\nsu -c 'id -u'\n\
        dnf install vlc\nls /nonexistent-wolos-hint\ncp /nonexistent-wolos-hint /usr/x\n\n\ntrue\n";

    fn check_log(shell: &str, log: &str, err: &str) {
        let calls: Vec<&str> = log.lines().collect();
        let want = [
            "sudo|-i|",
            "sudo|-i|",
            "sudo|-iu|bob|",
            "sudo|-iu|bob|",
            "sudo|sh|-c|id -u|",
            "wol|hint|dnf install vlc|--not-found|",
            "wol|hint|cp /nonexistent-wolos-hint /usr/x|1|",
        ];
        assert_eq!(calls, want, "{shell} calls:\n{log}\nstderr:\n{err}");
    }

    #[test]
    fn bash_wires_su_not_found_and_the_hook() {
        if Command::new("bash").arg("-c").arg(":").status().is_err() {
            eprintln!("bash not on this host: skipped");
            return;
        }
        let d = fakes("bash");
        let rc = share("bash/bashrc");
        let (log, err) = run_shell(
            "bash",
            &["--norc", "--noprofile", "-i"],
            &d,
            &format!(". {}\n{SCRIPT}", rc.display()),
        );
        check_log("bash", &log, &err);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn zsh_wires_su_not_found_and_the_hook() {
        if Command::new("zsh").arg("-fc").arg(":").status().is_err() {
            eprintln!("zsh not on this host: skipped");
            return;
        }
        let d = fakes("zsh");
        let rc = share("zsh/zshrc");
        let (log, err) = run_shell(
            "zsh",
            &["-f", "-i"],
            &d,
            &format!("source {}\n{SCRIPT}", rc.display()),
        );
        check_log("zsh", &log, &err);
        let _ = std::fs::remove_dir_all(&d);
    }
}
