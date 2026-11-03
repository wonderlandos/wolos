// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol script` (ADR 0041, docs/modules/scripting.md): one way to check and run a script whatever
//! it is written in. A shell script and a Ruby script are the same thing to WolOS: the kind comes
//! from the shebang (else the file name), the program is started with an argv (never a shell
//! string), as the person and never as root, with a clean environment (no tokens or keys from
//! the calling shell) and `WOL_HELPER=<name>`, so every `wol` change it asks for goes through the
//! helper gate (policy, approval sheet, audit log; docs/modules/automation.md). `wol script on
//! <host>` sends the same file over ssh on stdin and runs it there the same way. Each run is one
//! record in the helper audit log. One table, [`KINDS`], holds what differs between languages:
//! sh, bash, Ruby and pwsh (`.ps1`, the Console module's object shell).
use crate::cli::Invocation;
use crate::run::{Env, emit_json, fail};
use serde_json::{Value, json};
use shared::errors::ErrorCode;
use std::path::Path;
use wol_core::automation::{Outcome, audit};

/// One script language: how to recognise it, run it, read it from stdin and check it.
#[derive(Clone, Copy, Debug)]
pub struct Kind {
    /// `sh`, `bash`, `ruby`, `ps1`.
    pub id: &'static str,
    /// Interpreter names a shebang may give (`#!/usr/bin/ruby`, `#!/usr/bin/env ruby`).
    pub interpreters: &'static [&'static str],
    /// File name endings used when there is no shebang.
    pub extensions: &'static [&'static str],
    /// The interpreter on a WolOS system.
    pub program: &'static str,
    /// Arguments before the script's file when it runs here (empty for sh, bash, ruby; pwsh
    /// takes `-File` and no profile, as a shebang start does not read one either).
    pub run_args: &'static [&'static str],
    /// Arguments before the script's own when the program reads the script from stdin.
    pub stdin_args: &'static [&'static str],
    /// Syntax check without running anything (`sh -n`, `ruby -wc`, pwsh's parser through
    /// `/usr/share/wolos/pwsh/check.ps1`), given before the file.
    pub syntax: &'static [&'static str],
    /// Linter used when it is installed: program and arguments before the file.
    pub lint: Option<(&'static str, &'static [&'static str])>,
    /// The only checks the linter runs (`--only a,b`); empty = the linter's own choice.
    pub lint_only: &'static [&'static str],
}

/// RuboCop cops `wol script check` and tools/ruby/lint.sh run (an explicit list, never the
/// release's defaults, so a RuboCop update cannot add new failures; tools/ruby/rubocop.yml holds
/// the same list, `rules_ruby.rs` keeps the two equal).
pub const RUBOCOP_COPS: &[&str] = &[
    "Lint/Debugger",
    "Lint/RescueException",
    "Lint/ShadowingOuterLocalVariable",
    "Lint/SuppressedException",
    "Lint/Syntax",
    "Lint/UnreachableCode",
    "Lint/UnusedMethodArgument",
    "Lint/UselessAssignment",
    "Lint/Void",
    "Security/Eval",
    "Security/JSONLoad",
    "Security/MarshalLoad",
    "Security/Open",
    "Security/YAMLLoad",
    "Style/FrozenStringLiteralComment",
];

pub const KINDS: &[Kind] = &[
    Kind {
        id: "sh",
        interpreters: &["sh", "dash"],
        extensions: &[".sh"],
        program: "/bin/sh",
        run_args: &[],
        stdin_args: &["-s", "--"],
        syntax: &["-n"],
        lint: Some(("shellcheck", &["-S", "error", "-s", "sh"])),
        lint_only: &[],
    },
    Kind {
        id: "bash",
        interpreters: &["bash"],
        extensions: &[".bash"],
        program: "/bin/bash",
        run_args: &[],
        stdin_args: &["-s", "--"],
        syntax: &["-n"],
        lint: Some(("shellcheck", &["-S", "error", "-s", "bash"])),
        lint_only: &[],
    },
    Kind {
        id: "ruby",
        interpreters: &["ruby"],
        extensions: &[".rb"],
        program: "/usr/bin/ruby",
        run_args: &[],
        stdin_args: &["-"],
        syntax: &["-wc"],
        lint: Some((
            "rubocop",
            &[
                "--force-default-config",
                "--disable-pending-cops",
                "--format",
                "emacs",
            ],
        )),
        lint_only: RUBOCOP_COPS,
    },
    // pwsh is the person's own (Console module, wopkg recipe, ~/.local/bin); the shim at
    // /usr/lib/wolos/pwsh starts it, or says where it comes from and exits 127. No linter: the
    // analyzer is a pwsh module, not a program `--version` can find.
    Kind {
        id: "ps1",
        interpreters: &["pwsh"],
        extensions: &[".ps1"],
        program: "/usr/lib/wolos/pwsh",
        run_args: &[PWSH_BATCH[0], PWSH_BATCH[1], PWSH_BATCH[2], "-File"],
        stdin_args: &[
            PWSH_BATCH[0],
            PWSH_BATCH[1],
            PWSH_BATCH[2],
            "-CommandWithArgs",
            PWSH_FROM_STDIN,
        ],
        syntax: &[
            PWSH_BATCH[0],
            PWSH_BATCH[1],
            PWSH_BATCH[2],
            "-File",
            "/usr/share/wolos/pwsh/check.ps1",
        ],
        lint: None,
        lint_only: &[],
    },
];

/// pwsh options every `ps1` start uses: no banner, no profile, never a prompt.
const PWSH_BATCH: [&str; 3] = ["-NoLogo", "-NoProfile", "-NonInteractive"];

/// What `wol script on` runs on the other computer for a `.ps1` file: pwsh reads the script from
/// stdin, makes it one script block and calls it with the script's arguments (`$args` and
/// `param()` work as with `-File`; `$PSScriptRoot` is empty, there is no file).
pub const PWSH_FROM_STDIN: &str = "& ([scriptblock]::Create([Console]::In.ReadToEnd())) @args";

/// The fixed `PATH` a script gets.
pub const PATH: &str = "/usr/local/bin:/usr/bin";

/// Variables a script keeps from the caller (everything else, tokens and keys included, is
/// dropped): who and where the person is, language, terminal, and what reaches the desktop
/// session (the approval sheet needs `XDG_RUNTIME_DIR`).
pub const KEPT_ENV: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "TZ",
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "DBUS_SESSION_BUS_ADDRESS",
];

/// The interpreter a `#!` line names: its last path part, or the program after `env`
/// (`#!/usr/bin/env -S ruby -w` gives `ruby`). A version suffix is dropped (`ruby3.4`).
pub fn shebang_interpreter(first_line: &str) -> Option<String> {
    let rest = first_line.strip_prefix("#!")?.trim();
    let mut words = rest.split_whitespace();
    let base = |w: &str| w.rsplit('/').next().unwrap_or(w).to_string();
    let mut prog = base(words.next()?);
    if prog == "env" {
        prog = base(words.find(|w| !w.starts_with('-') && !w.contains('='))?);
    }
    let trimmed = prog.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    Some(if trimmed.is_empty() {
        prog
    } else {
        trimmed.to_string()
    })
}

/// The kind of `name` with `text`: by its shebang, else by its file name ending.
/// `Err` says why there is none (one sentence).
pub fn kind_of(name: &str, text: &str) -> Result<&'static Kind, String> {
    let first = text.lines().next().unwrap_or("");
    if let Some(i) = shebang_interpreter(first) {
        return KINDS
            .iter()
            .find(|k| k.interpreters.contains(&i.as_str()))
            .ok_or_else(|| format!("{name} is a {i} script; wol script runs {}.", kinds_list()));
    }
    let lower = name.to_ascii_lowercase();
    KINDS
        .iter()
        .find(|k| k.extensions.iter().any(|e| lower.ends_with(e)))
        .ok_or_else(|| {
            format!(
                "{name} has no #! line and no known ending ({}); wol script runs {}.",
                KINDS
                    .iter()
                    .flat_map(|k| k.extensions.iter().copied())
                    .collect::<Vec<_>>()
                    .join(" "),
                kinds_list()
            )
        })
}

fn kinds_list() -> String {
    let ids: Vec<&str> = KINDS.iter().map(|k| k.id).collect();
    match ids.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => ids.join(""),
    }
}

/// The helper name of a script: its file name without the ending, letters, digits, `.`, `_`, `-`
/// only (`backup.rb` -> `backup`).
pub fn helper_name(file: &str) -> String {
    let base = Path::new(file)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let stem = base.rsplit_once('.').map_or(base, |(s, _)| s);
    let clean: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect();
    if clean.is_empty() || clean.starts_with(['.', '-']) {
        "script".into()
    } else {
        clean
    }
}

/// A host for ssh: a name, an address or `user@host`; never something ssh would read as an
/// option.
pub fn host_ok(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && !host.starts_with('-')
        && host.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | ':' | '[' | ']')
        })
}

/// A path the interpreter cannot take for an option.
fn as_operand(file: &str) -> String {
    if file.starts_with('-') {
        format!("./{file}")
    } else {
        file.to_string()
    }
}

/// argv that runs `file` here: `env -i` with [`KEPT_ENV`], [`PATH`] and `WOL_HELPER`, then the
/// interpreter, its [`Kind::run_args`], the file and the script's arguments.
pub fn local_argv(env: &dyn Env, kind: &Kind, file: &str, args: &[String]) -> Vec<String> {
    let mut v = vec!["/usr/bin/env".to_string(), "-i".into()];
    for k in KEPT_ENV {
        if let Some(x) = env.var(k) {
            v.push(format!("{k}={x}"));
        }
    }
    v.push(format!("PATH={PATH}"));
    v.push(format!("WOL_HELPER={}", helper_name(file)));
    v.push(kind.program.into());
    v.extend(kind.run_args.iter().map(|a| a.to_string()));
    v.push(as_operand(file));
    v.extend(args.iter().cloned());
    v
}

/// The command line the other computer's shell runs for `wol script on`: the same clean
/// environment (its own `HOME`, `USER`, `LANG`), then the interpreter reading the script from
/// stdin. Every word is single-quoted.
pub fn remote_command(kind: &Kind, file: &str, args: &[String]) -> String {
    use wol_core::sessions::sh_quote;
    let mut words = vec![
        "exec".to_string(),
        "/usr/bin/env".into(),
        "-i".into(),
        "HOME=\"$HOME\"".into(),
        "USER=\"$USER\"".into(),
        "LOGNAME=\"$LOGNAME\"".into(),
        "LANG=\"${LANG:-C.UTF-8}\"".into(),
        sh_quote(&format!("PATH={PATH}")),
        sh_quote(&format!("WOL_HELPER={}", helper_name(file))),
        sh_quote(kind.program),
    ];
    words.extend(kind.stdin_args.iter().map(|a| sh_quote(a)));
    words.extend(args.iter().map(|a| sh_quote(a)));
    words.join(" ")
}

/// argv for `wol script on`: ssh without a terminal, no password prompts (keys only).
pub fn ssh_argv(host: &str, command: &str) -> Vec<String> {
    [
        "ssh",
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=20",
        "--",
        host,
        command,
    ]
    .map(String::from)
    .to_vec()
}

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json");
    let listing_json = json || !env.stdout_tty();
    let file = inv.get("file").unwrap_or_default().to_string();
    let args: Vec<String> = inv.all("args").iter().map(|s| s.to_string()).collect();
    match inv.action {
        "script.kinds" => kinds(env, listing_json),
        "script.check" => check(env, &file, json),
        "script.run" => run(env, &file, &args, None),
        "script.on" => run(env, &file, &args, inv.get("host")),
        other => fail(env, ErrorCode::Failed, &format!("no handler for {other}")),
    }
}

fn kinds(env: &mut dyn Env, json: bool) -> i32 {
    if json {
        let v: Vec<Value> = KINDS
            .iter()
            .map(|k| {
                json!({
                    "id": k.id,
                    "interpreters": k.interpreters,
                    "extensions": k.extensions,
                    "program": k.program,
                    "lint": k.lint.map(|l| l.0),
                })
            })
            .collect();
        emit_json(env, &Value::Array(v));
    } else {
        for k in KINDS {
            env.out(&format!(
                "{:<6} {:<8} {:<19} {}\n",
                k.id,
                k.extensions.join(" "),
                k.program,
                k.lint.map_or("-", |l| l.0)
            ));
        }
    }
    0
}

/// The script's text and kind, or the exit code after saying why not.
fn load(env: &mut dyn Env, file: &str) -> Result<(String, &'static Kind), i32> {
    if file.is_empty() {
        return Err(fail(env, ErrorCode::Usage, "give a script file"));
    }
    let Some(text) = env.read_file(Path::new(file)) else {
        return Err(fail(
            env,
            ErrorCode::NotFound,
            &format!("{file}: no such script (or it is not text)"),
        ));
    };
    match kind_of(file, &text) {
        Ok(k) => Ok((text, k)),
        Err(why) => Err(fail(env, ErrorCode::Unsupported, &why)),
    }
}

/// `wol script check`: parses with the interpreter (nothing runs), then the linter when it is
/// installed. One sentence per problem; the tools' own messages go to stderr.
fn check(env: &mut dyn Env, file: &str, json: bool) -> i32 {
    let (_, kind) = match load(env, file) {
        Ok(x) => x,
        Err(code) => return code,
    };
    let operand = as_operand(file);
    let mut problems: Vec<String> = Vec::new();
    let mut argv: Vec<String> = vec![kind.program.into()];
    argv.extend(kind.syntax.iter().map(|s| s.to_string()));
    argv.push(operand.clone());
    // In JSON mode stdout carries the answer only: the tools' output goes to stderr.
    let run = |env: &mut dyn Env, argv: &[String]| {
        if json {
            env.exec_quiet(argv)
        } else {
            env.exec(argv)
        }
    };
    match run(env, &argv) {
        Ok(0) => {}
        // 127: the program is a launcher whose interpreter is missing (the pwsh shim says so).
        Ok(127) | Err(_) => problems.push(format!(
            "{} is not installed, so {file} cannot be checked.",
            kind.program
        )),
        Ok(_) => problems.push(format!(
            "{file} does not parse as {}: the lines above say where.",
            kind.id
        )),
    }
    let mut linted = None;
    if let Some((tool, targs)) = kind.lint
        && problems.is_empty()
        && env.capture(&[tool, "--version"]).is_some()
    {
        let mut argv: Vec<String> = vec![tool.into()];
        argv.extend(targs.iter().map(|s| s.to_string()));
        if !kind.lint_only.is_empty() {
            argv.push("--only".into());
            argv.push(kind.lint_only.join(","));
        }
        argv.push(operand);
        let ok = matches!(run(env, &argv), Ok(0));
        if !ok {
            problems.push(format!(
                "{tool} found problems in {file}: the lines above list them."
            ));
        }
        linted = Some(tool);
    }
    let ok = problems.is_empty();
    if json {
        emit_json(
            env,
            &json!({"file": file, "kind": kind.id, "ok": ok, "lint": linted, "problems": problems}),
        );
    } else if ok {
        let how = linted.map_or(String::new(), |t| format!(" and {t} is clean"));
        env.out(&format!("{file}: {} script, parses{how}\n", kind.id));
    } else {
        for p in &problems {
            env.out(&format!("{p}\n"));
        }
    }
    if ok { 0 } else { ErrorCode::Invalid.exit() }
}

/// `wol script run` (here) and `wol script on <host>` (over ssh): same kinds, same clean
/// environment, same helper name, one audit record with the exit code (the script's arguments
/// are not recorded: they may hold secrets).
fn run(env: &mut dyn Env, file: &str, args: &[String], host: Option<&str>) -> i32 {
    if env.is_root() {
        return fail(
            env,
            ErrorCode::Denied,
            "wol script runs a script as you, never as root (ADR 0041); run it without sudo",
        );
    }
    if let Some(h) = host
        && !host_ok(h)
    {
        return fail(
            env,
            ErrorCode::Usage,
            &format!("'{h}' is not a host name, an address or user@host"),
        );
    }
    let (text, kind) = match load(env, file) {
        Ok(x) => x,
        Err(code) => return code,
    };
    let name = helper_name(file);
    let (argv, result) = match host {
        None => {
            let argv = local_argv(env, kind, file, args);
            let r = env.exec(&argv);
            (argv, r)
        }
        Some(h) => {
            let argv = ssh_argv(h, &remote_command(kind, file, args));
            let r = env.exec_input(&argv, &text);
            (argv, r)
        }
    };
    let (code, outcome, detail) = match result {
        Ok(c) => (
            c,
            if c == 0 { Outcome::Ok } else { Outcome::Failed },
            format!("exit {c}"),
        ),
        Err(e) => {
            let msg = format!("{}: {e}", argv[0]);
            (
                fail(env, ErrorCode::Unavailable, &msg),
                Outcome::Failed,
                msg,
            )
        }
    };
    let (tool, via, rec_args) = match host {
        None => ("script run", "cli", vec![file.to_string()]),
        Some(h) => ("script on", "ssh", vec![h.to_string(), file.to_string()]),
    };
    let rec_args: Vec<&str> = rec_args.iter().map(String::as_str).collect();
    let rec = crate::verbs::record(
        &format!("script:{name}"),
        via,
        tool,
        &rec_args,
        outcome,
        &detail,
        env.now_secs(),
    );
    let _ = audit::append(&env.state_home(), &rec);
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shebangs_name_their_interpreter() {
        for (line, want) in [
            ("#!/bin/sh", Some("sh")),
            ("#!/bin/sh -eu", Some("sh")),
            ("#!/usr/bin/env bash", Some("bash")),
            ("#!/usr/bin/ruby", Some("ruby")),
            ("#!/usr/bin/env ruby", Some("ruby")),
            ("#!/usr/bin/env -S ruby -w", Some("ruby")),
            ("#!/usr/bin/ruby3.4", Some("ruby")),
            ("#!/usr/bin/env pwsh", Some("pwsh")),
            ("#!/usr/bin/env -S pwsh -NoProfile", Some("pwsh")),
            ("#!/usr/lib/wolos/pwsh", Some("pwsh")),
            ("#! /usr/bin/python3", Some("python")),
            ("# a comment", None),
            ("#!", None),
        ] {
            assert_eq!(shebang_interpreter(line).as_deref(), want, "{line}");
        }
    }

    #[test]
    fn the_shebang_wins_over_the_name() {
        assert_eq!(kind_of("x.sh", "#!/usr/bin/ruby\n").unwrap().id, "ruby");
        assert_eq!(kind_of("x.rb", "puts 1\n").unwrap().id, "ruby");
        assert_eq!(kind_of("x.SH", "echo\n").unwrap().id, "sh");
        assert_eq!(kind_of("x", "#!/bin/bash\n").unwrap().id, "bash");
        assert_eq!(kind_of("x.PS1", "Get-Date\n").unwrap().id, "ps1");
        assert_eq!(kind_of("tidy", "#!/usr/bin/env pwsh\n").unwrap().id, "ps1");
        assert_eq!(kind_of("x.ps1", "#!/bin/sh\n").unwrap().id, "sh");
        let e = kind_of("x.py", "#!/usr/bin/python3\n").unwrap_err();
        assert!(
            e.contains("python script") && e.contains("sh, bash, ruby and ps1"),
            "{e}"
        );
        let e = kind_of("notes.txt", "hello\n").unwrap_err();
        assert!(e.contains(".rb"), "{e}");
    }

    #[test]
    fn helper_names_are_plain() {
        assert_eq!(helper_name("/home/a/backup.rb"), "backup");
        assert_eq!(helper_name("tidy.sh"), "tidy");
        assert_eq!(helper_name("run"), "run");
        assert_eq!(helper_name("we ird$name.rb"), "weirdname");
        assert_eq!(helper_name("-x.rb"), "script");
        assert_eq!(helper_name(".rb"), "script");
    }

    #[test]
    fn hosts_cannot_be_options() {
        for h in ["box", "klee@box.lan", "10.0.0.2", "[fe80::1]"] {
            assert!(host_ok(h), "{h}");
        }
        for h in ["", "-oProxyCommand=x", "a b", "a;b", "$(x)"] {
            assert!(!host_ok(h), "{h}");
        }
    }

    #[test]
    fn the_remote_command_quotes_every_word() {
        let ruby = &KINDS[2];
        let c = remote_command(ruby, "/x/it's.rb", &["a b".into(), "'; rm -rf ~".into()]);
        assert!(c.starts_with("exec /usr/bin/env -i HOME=\"$HOME\""), "{c}");
        assert!(
            c.contains("'WOL_HELPER=its' '/usr/bin/ruby' '-' 'a b' ''\\''; rm -rf ~'"),
            "{c}"
        );
        let sh = &KINDS[0];
        assert!(remote_command(sh, "t.sh", &[]).ends_with("'/bin/sh' '-s' '--'"));
        let ps1 = &KINDS[3];
        let c = remote_command(ps1, "t.ps1", &["x".into()]);
        assert!(
            c.ends_with(
                "'/usr/lib/wolos/pwsh' '-NoLogo' '-NoProfile' '-NonInteractive' \
                 '-CommandWithArgs' '& ([scriptblock]::Create([Console]::In.ReadToEnd())) @args' 'x'"
            ),
            "{c}"
        );
    }

    #[test]
    fn the_cop_list_is_sorted_and_unique() {
        let mut sorted = RUBOCOP_COPS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, RUBOCOP_COPS);
    }
}
