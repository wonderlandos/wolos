// SPDX-License-Identifier: BSD-2-Clause
//! Sessions (ADR 0032 "Sessions", docs/modules/modules.md): the window set a person can reopen at
//! the next login. One record per terminal or agent window, `{workspace, cwd, kind, command,
//! title, terminal}`, saved to `$XDG_STATE_HOME/wolos/sessions.json` by `wol sessions save` (the
//! shell runs it while the session lives) and relaunched by `wol sessions restore` in the same
//! workspace. No process survives a reboot; the place, the directory and the conversation pointer
//! (`claude --continue`, a named zellij session) do.
//!
//! Invariant: nothing read from the file is ever run as text. `command` is matched against a
//! fixed list ([`Command`]) and the relaunch argv is rebuilt from it, the directory is checked
//! ([`valid_cwd`]) and every word handed to the compositor's `exec` (which runs `/bin/sh -c`) is
//! single-quoted ([`sh_quote`]). Pure: callers pass the compositor and `/proc` facts they read.
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

/// Saved window set, relative to the state home.
pub const FILE: &str = "wolos/sessions.json";
/// Largest sessions file read (bytes); a bigger one is treated as empty.
pub const MAX_BYTES: usize = 256 * 1024;
/// Most windows saved and restored.
pub const MAX_WINDOWS: usize = 64;
/// Longest directory or title kept (bytes).
pub const MAX_FIELD: usize = 1024;
/// Format version written in the file; another version reads as nothing saved.
pub const VERSION: u32 = 1;
/// Terminal window classes (compositor `class`) whose windows are saved: foot (default), ghostty,
/// kitty (ADR 0032 "Console").
pub const TERMINALS: [&str; 3] = ["foot", "com.mitchellh.ghostty", "kitty"];
/// How deep under the terminal the foreground program is looked for (terminal -> shell -> program).
pub const MAX_DEPTH: usize = 4;

/// What a window was: a plain terminal (maybe a zellij client) or a coding agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Terminal,
    Agent,
}

/// One saved window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Compositor workspace id; 0 or below (special, unknown) = wherever it opens.
    pub workspace: i64,
    /// Working directory of the shell in the window; absolute.
    pub cwd: String,
    pub kind: Kind,
    /// The foreground program, one of [`Command`]'s words: "" (the shell), `zellij`,
    /// `zellij attach <name>`, `claude --continue`.
    pub command: String,
    /// Window title when saved (shown by `wol sessions list`).
    pub title: String,
    /// Terminal class from [`TERMINALS`]; anything else reopens in foot.
    #[serde(default)]
    pub terminal: String,
}

/// The file: format version, when it was written (seconds since the epoch), the windows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub version: u32,
    pub saved_at: u64,
    pub windows: Vec<Entry>,
}

/// A foreground program WolOS knows how to bring back. Everything else reopens as a shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Shell,
    /// zellij without a name: a new session.
    Zellij,
    /// `zellij attach --create <name>`: the same named session (zellij resurrects it).
    ZellijAttach(String),
    /// `claude --continue`: the last conversation in this directory.
    ClaudeContinue,
}

impl Command {
    /// The word saved in [`Entry::command`].
    pub fn word(&self) -> String {
        match self {
            Command::Shell => String::new(),
            Command::Zellij => "zellij".into(),
            Command::ZellijAttach(n) => format!("zellij attach {n}"),
            Command::ClaudeContinue => "claude --continue".into(),
        }
    }

    /// Parse a saved word; None for anything not in the list (the window is then skipped).
    pub fn parse(word: &str) -> Option<Command> {
        match word {
            "" => Some(Command::Shell),
            "zellij" => Some(Command::Zellij),
            "claude --continue" => Some(Command::ClaudeContinue),
            w => {
                let name = w.strip_prefix("zellij attach ")?;
                valid_zellij_name(name).then(|| Command::ZellijAttach(name.into()))
            }
        }
    }

    /// The program's argv inside the terminal; empty for the shell itself.
    pub fn argv(&self) -> Vec<String> {
        match self {
            Command::Shell => Vec::new(),
            Command::Zellij => vec!["zellij".into()],
            Command::ZellijAttach(n) => {
                vec![
                    "zellij".into(),
                    "attach".into(),
                    "--create".into(),
                    n.clone(),
                ]
            }
            Command::ClaudeContinue => vec!["claude".into(), "--continue".into()],
        }
    }
}

/// zellij session names WolOS saves: `[A-Za-z0-9_.-]`, 1..=64 bytes, not starting with `-`.
/// False for anything else.
pub fn valid_zellij_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && !n.starts_with('-')
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// A directory WolOS reopens a window in: absolute, no `..`, no control characters, at most
/// [`MAX_FIELD`] bytes. False for anything else (the window is then skipped).
pub fn valid_cwd(cwd: &str) -> bool {
    cwd.len() <= MAX_FIELD
        && cwd.starts_with('/')
        && !cwd.chars().any(char::is_control)
        && !Path::new(cwd)
            .components()
            .any(|c| matches!(c, Component::ParentDir))
}

/// One window as `hyprctl -j clients` reports it (the fields sessions need).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Window {
    pub class: String,
    pub title: String,
    pub workspace: WorkspaceRef,
    pub pid: i64,
}

/// `workspace` of a compositor client.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct WorkspaceRef {
    pub id: i64,
}

/// Largest `hyprctl -j clients` reply read (bytes); bigger = no windows.
pub const MAX_CLIENTS_BYTES: usize = 4 * 1024 * 1024;

/// `hyprctl -j clients` -> windows; an unreadable or oversized reply is no windows.
pub fn parse_clients(json: &str) -> Vec<Window> {
    if json.len() > MAX_CLIENTS_BYTES {
        return Vec::new();
    }
    serde_json::from_str(json).unwrap_or_default()
}

/// A process as `/proc` shows it: argv, working directory, children.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Proc {
    pub argv: Vec<String>,
    pub cwd: String,
    pub children: Vec<u32>,
}

/// `/proc/<pid>/cmdline` (NUL-separated) -> argv; empty for an empty or kernel process.
pub fn parse_cmdline(raw: &str) -> Vec<String> {
    raw.split('\0')
        .filter(|a| !a.is_empty())
        .take(256)
        .map(clip)
        .collect()
}

/// `/proc/<pid>/task/<pid>/children` (space-separated pids) -> pids, at most [`MAX_WINDOWS`].
pub fn parse_children(raw: &str) -> Vec<u32> {
    raw.split_ascii_whitespace()
        .filter_map(|p| p.parse().ok())
        .take(MAX_WINDOWS)
        .collect()
}

fn base(arg0: &str) -> &str {
    arg0.rsplit('/').next().unwrap_or(arg0)
}

/// What a program's argv is, when WolOS can bring it back.
fn recognise(argv: &[String]) -> Option<(Kind, Command)> {
    let first = argv.first()?;
    match base(first) {
        "claude" => Some((Kind::Agent, Command::ClaudeContinue)),
        "zellij" => {
            // `zellij attach <name>`, `zellij -s <name>`, `zellij --session <name>`.
            let mut it = argv.iter().skip(1);
            while let Some(a) = it.next() {
                if matches!(a.as_str(), "attach" | "a" | "-s" | "--session") {
                    let name = it.find(|x| !x.starts_with('-'));
                    return Some((
                        Kind::Terminal,
                        name.filter(|n| valid_zellij_name(n))
                            .map_or(Command::Zellij, |n| Command::ZellijAttach(n.clone())),
                    ));
                }
            }
            Some((Kind::Terminal, Command::Zellij))
        }
        _ => None,
    }
}

/// The foreground program under `pid`, breadth first up to [`MAX_DEPTH`].
fn foreground(pid: u32, procs: &dyn Fn(u32) -> Option<Proc>) -> Option<(Kind, Command)> {
    let mut level = vec![pid];
    for _ in 0..MAX_DEPTH {
        let mut next = Vec::new();
        for p in level {
            let Some(pr) = procs(p) else { continue };
            if let Some(found) = recognise(&pr.argv) {
                return Some(found);
            }
            next.extend(pr.children.iter().take(MAX_WINDOWS).copied());
        }
        if next.is_empty() {
            break;
        }
        next.truncate(MAX_WINDOWS);
        level = next;
    }
    None
}

fn clip(s: &str) -> String {
    let mut end = s.len().min(MAX_FIELD);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// The entry for one window, or None when it is not a terminal or its directory is unknown.
/// The directory is the shell's (the terminal's first child), else the terminal's own.
pub fn entry_for(w: &Window, procs: &dyn Fn(u32) -> Option<Proc>) -> Option<Entry> {
    if !TERMINALS.contains(&w.class.as_str()) {
        return None;
    }
    let pid = u32::try_from(w.pid).ok().filter(|p| *p > 0)?;
    let term = procs(pid)?;
    let shell = term.children.first().and_then(|c| procs(*c));
    let cwd = shell
        .as_ref()
        .map(|s| s.cwd.clone())
        .filter(|c| valid_cwd(c))
        .or_else(|| Some(term.cwd.clone()).filter(|c| valid_cwd(c)))?;
    let (kind, cmd) = term
        .children
        .first()
        .and_then(|c| foreground(*c, procs))
        .unwrap_or((Kind::Terminal, Command::Shell));
    Some(Entry {
        workspace: w.workspace.id,
        cwd,
        kind,
        command: cmd.word(),
        title: clip(&w.title),
        terminal: w.class.clone(),
    })
}

/// Every saveable window, at most [`MAX_WINDOWS`], in workspace order (stable within one).
pub fn collect(windows: &[Window], procs: &dyn Fn(u32) -> Option<Proc>) -> Vec<Entry> {
    let mut v: Vec<Entry> = windows
        .iter()
        .filter_map(|w| entry_for(w, procs))
        .take(MAX_WINDOWS)
        .collect();
    v.sort_by_key(|e| e.workspace);
    v
}

/// The file's text for `windows` saved at `now`; an empty string only if serde fails.
pub fn to_json(windows: &[Entry], now: u64) -> String {
    let s = Saved {
        version: VERSION,
        saved_at: now,
        windows: windows.iter().take(MAX_WINDOWS).cloned().collect(),
    };
    serde_json::to_string_pretty(&s).map_or_else(|_| String::new(), |t| t + "\n")
}

/// Read the file's text: oversized, malformed or another version = nothing saved; windows past
/// [`MAX_WINDOWS`] and entries [`relaunch_argv`] would refuse are dropped.
pub fn parse(text: &str) -> Saved {
    if text.len() > MAX_BYTES {
        return Saved::default();
    }
    let Ok(mut s) = serde_json::from_str::<Saved>(text) else {
        return Saved::default();
    };
    if s.version != VERSION {
        return Saved::default();
    }
    s.windows.truncate(MAX_WINDOWS);
    s.windows.retain(|e| relaunch_argv(e).is_some());
    s
}

/// The argv that reopens `e`: the terminal in its directory running the saved program. None when
/// the directory or the program is not one WolOS accepts.
pub fn relaunch_argv(e: &Entry) -> Option<Vec<String>> {
    if !valid_cwd(&e.cwd) || e.title.len() > MAX_FIELD {
        return None;
    }
    let prog = Command::parse(&e.command)?.argv();
    let ghostty = e.terminal == "com.mitchellh.ghostty";
    let mut argv: Vec<String> = match e.terminal.as_str() {
        "com.mitchellh.ghostty" => vec!["ghostty".into(), format!("--working-directory={}", e.cwd)],
        "kitty" => vec!["kitty".into(), "--directory".into(), e.cwd.clone()],
        _ => vec!["foot".into(), "--working-directory".into(), e.cwd.clone()],
    };
    if !prog.is_empty() {
        if ghostty {
            argv.push("-e".into());
        }
        argv.extend(prog);
    }
    Some(argv)
}

/// POSIX single quotes: the word reaches the program unchanged whatever it contains.
pub fn sh_quote(w: &str) -> String {
    format!("'{}'", w.replace('\'', r"'\''"))
}

/// The `hyprctl dispatch exec` argument that reopens `e` in its workspace without taking focus;
/// None when [`relaunch_argv`] refuses it. Workspaces 1..=100 are kept, others open where the
/// person is.
pub fn hypr_exec(e: &Entry) -> Option<String> {
    let argv = relaunch_argv(e)?;
    let words: Vec<String> = argv.iter().map(|w| sh_quote(w)).collect();
    let rule = if (1..=100).contains(&e.workspace) {
        format!("[workspace {} silent] ", e.workspace)
    } else {
        String::new()
    };
    Some(format!("{rule}{}", words.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn procs(pid: u32) -> Option<Proc> {
        let p = |argv: &[&str], cwd: &str, children: &[u32]| Proc {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            cwd: cwd.into(),
            children: children.to_vec(),
        };
        Some(match pid {
            100 => p(&["foot"], "/home/a", &[101]),
            101 => p(&["-zsh"], "/home/a/Work/site", &[102]),
            102 => p(&["/home/a/.local/bin/claude"], "/home/a/Work/site", &[]),
            200 => p(&["foot"], "/home/a", &[201]),
            201 => p(&["zsh"], "/home/a/src", &[202]),
            202 => p(&["zellij", "attach", "work"], "/home/a/src", &[]),
            300 => p(&["foot"], "/home/a", &[301]),
            301 => p(&["zsh"], "/tmp/../etc", &[]),
            400 => p(&["kitty"], "/home/a", &[401]),
            401 => p(&["bash"], "/home/a/notes", &[402]),
            402 => p(&["vim", "x"], "/home/a/notes", &[]),
            _ => return None,
        })
    }

    fn win(class: &str, pid: i64, ws: i64) -> Window {
        Window {
            class: class.into(),
            title: format!("{class} {pid}"),
            workspace: WorkspaceRef { id: ws },
            pid,
        }
    }

    #[test]
    fn terminals_agents_and_zellij_are_recognised() {
        let ws = [
            win("foot", 100, 2),
            win("foot", 200, 1),
            win("firefox", 900, 1),
            win("foot", 300, 3),
            win("kitty", 400, 4),
            win("foot", -1, 5),
        ];
        let v = collect(&ws, &procs);
        assert_eq!(v.len(), 4, "{v:?}");
        assert_eq!(v[0].command, "zellij attach work");
        assert_eq!((v[0].workspace, v[0].cwd.as_str()), (1, "/home/a/src"));
        assert_eq!(v[1].kind, Kind::Agent);
        assert_eq!(v[1].command, "claude --continue");
        assert_eq!(v[1].cwd, "/home/a/Work/site");
        // A shell directory with `..` falls back to the terminal's own directory.
        assert_eq!((v[2].workspace, v[2].cwd.as_str()), (3, "/home/a"));
        // An unknown program (vim) reopens as the shell, in its directory.
        assert_eq!((v[3].kind, v[3].command.as_str()), (Kind::Terminal, ""));
        assert_eq!(
            (v[3].terminal.as_str(), v[3].cwd.as_str()),
            ("kitty", "/home/a/notes")
        );
    }

    #[test]
    fn round_trip_and_relaunch() {
        let v = collect(&[win("foot", 100, 2), win("foot", 200, 1)], &procs);
        let s = parse(&to_json(&v, 42));
        assert_eq!((s.version, s.saved_at), (VERSION, 42));
        assert_eq!(s.windows, v);
        assert_eq!(
            relaunch_argv(&s.windows[1]).unwrap(),
            [
                "foot",
                "--working-directory",
                "/home/a/Work/site",
                "claude",
                "--continue"
            ]
        );
        assert_eq!(
            relaunch_argv(&s.windows[0]).unwrap(),
            [
                "foot",
                "--working-directory",
                "/home/a/src",
                "zellij",
                "attach",
                "--create",
                "work"
            ]
        );
        assert_eq!(
            hypr_exec(&s.windows[1]).unwrap(),
            "[workspace 2 silent] 'foot' '--working-directory' '/home/a/Work/site' 'claude' '--continue'"
        );
        let mut g = s.windows[1].clone();
        g.terminal = "com.mitchellh.ghostty".into();
        assert_eq!(
            relaunch_argv(&g).unwrap(),
            [
                "ghostty",
                "--working-directory=/home/a/Work/site",
                "-e",
                "claude",
                "--continue"
            ]
        );
    }

    #[test]
    fn saved_text_is_never_run() {
        let e = |cwd: &str, command: &str| Entry {
            workspace: 1,
            cwd: cwd.into(),
            kind: Kind::Terminal,
            command: command.into(),
            title: String::new(),
            terminal: "foot".into(),
        };
        // Programs outside the list, bad zellij names, relative or traversing directories.
        for (cwd, cmd) in [
            ("/home/a", "rm -rf ~"),
            ("/home/a", "zellij attach x;reboot"),
            ("/home/a", "zellij attach --help"),
            ("home/a", ""),
            ("/home/../etc", ""),
            ("/home/a\n", ""),
        ] {
            assert_eq!(relaunch_argv(&e(cwd, cmd)), None, "{cwd:?} {cmd:?}");
        }
        // A quote in a directory stays inside its word.
        let x = hypr_exec(&e("/home/a/it's $(reboot)", "")).unwrap();
        assert_eq!(
            x,
            r"[workspace 1 silent] 'foot' '--working-directory' '/home/a/it'\''s $(reboot)'"
        );
        // Special workspaces (negative ids) get no rule.
        let mut s = e("/home/a", "");
        s.workspace = -98;
        assert!(hypr_exec(&s).unwrap().starts_with("'foot'"));
    }

    #[test]
    fn bad_files_read_as_nothing_saved() {
        assert_eq!(parse("not json"), Saved::default());
        assert_eq!(parse(r#"{"version": 9, "windows": []}"#), Saved::default());
        assert_eq!(parse(&" ".repeat(MAX_BYTES + 1)), Saved::default());
        let one = r#"{"workspace":1,"cwd":"/h","kind":"terminal","command":"","title":"t"}"#;
        let many = format!(
            r#"{{"version":1,"saved_at":1,"windows":[{}]}}"#,
            vec![one; MAX_WINDOWS + 5].join(",")
        );
        let s = parse(&many);
        assert_eq!(s.windows.len(), MAX_WINDOWS);
        // A missing terminal reopens in foot.
        assert_eq!(relaunch_argv(&s.windows[0]).unwrap()[0], "foot");
        // An entry the relauncher refuses is dropped on read.
        let bad = r#"{"version":1,"windows":[{"workspace":1,"cwd":"rel","kind":"agent","command":"","title":""}]}"#;
        assert!(parse(bad).windows.is_empty());
    }

    #[test]
    fn proc_and_client_text_parse_with_bounds() {
        let j =
            r#"[{"class":"foot","title":"zsh","workspace":{"id":3,"name":"3"},"pid":77,"x":1}]"#;
        let w = parse_clients(j);
        assert_eq!((w[0].pid, w[0].workspace.id), (77, 3));
        assert!(parse_clients("{").is_empty());
        assert_eq!(
            parse_cmdline("zellij\0attach\0w\0"),
            ["zellij", "attach", "w"]
        );
        assert_eq!(parse_children("12 13 x 14\n"), [12, 13, 14]);
        assert!(valid_zellij_name("work-1.a_b"));
        for n in ["", "-x", "a b", "a/b", &"x".repeat(65)] {
            assert!(!valid_zellij_name(n), "{n}");
        }
        assert_eq!(clip(&"é".repeat(MAX_FIELD)).len(), MAX_FIELD);
    }
}
