// SPDX-License-Identifier: GPL-2.0-or-later
//! pwsh in zsh (docs/modules/pwsh.md): the WolOS zshrc loads /usr/share/wolos/zsh/pwsh.zsh when
//! the person's pwsh is installed; it adds `pw 'LINE'` (one pwsh line in a zsh pipeline, run by
//! /usr/share/wolos/pwsh/pw.ps1), runs `x.ps1` typed as a command, completes cmdlets and their
//! parameters from a list /usr/share/wolos/pwsh/commands.ps1 writes, and colours pwsh lines.
//! The zsh parts run in the real zsh with a fake pwsh; the pwsh parts in the real pwsh when one
//! is on PATH (both skipped with a message when missing, like image_pwsh.rs).
#[path = "support/siblings.rs"]
mod siblings;

use std::{fs, os::unix::fs::PermissionsExt, path::Path, path::PathBuf, process::Command};

const ZSH_FILE: &str = "desktop/overlay/usr/share/wolos/zsh/pwsh.zsh";
const ZSHRC: &str = "desktop/overlay/usr/share/wolos/zsh/zshrc";
const SHARE: &str = "desktop/overlay/usr/share/wolos/pwsh";
const RUNNER: &str = "desktop/overlay/usr/share/wolos/pwsh/pw.ps1";
const LISTER: &str = "desktop/overlay/usr/share/wolos/pwsh/commands.ps1";
const SETUP: &str = "desktop/overlay/usr/share/wolos/pwsh/profile.ps1";

fn repo(p: &str) -> PathBuf {
    siblings::root().join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn sandbox(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-pwzsh-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn exe(p: &Path, body: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
    fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
}

fn zsh() -> Option<&'static str> {
    let ok = Command::new("zsh").arg("-fc").arg(":").status();
    ok.is_ok_and(|s| s.success()).then_some("zsh")
}

/// The real pwsh on PATH (absolute), if any.
fn pwsh() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exe = std::env::split_paths(&path)
        .map(|d| d.join("pwsh"))
        .find(|p| p.is_file())?;
    let ok = Command::new(&exe)
        .args(["-NoProfile", "-NonInteractive", "-Command", "exit 0"])
        .status();
    ok.is_ok_and(|s| s.success()).then_some(exe)
}

#[test]
fn zshrc_loads_the_pwsh_file_only_with_pwsh_and_never_takes_ps() {
    let rc = read(ZSHRC);
    assert!(
        rc.contains("if [[ -x $HOME/.local/bin/pwsh ]] || (( $+commands[pwsh] )); then"),
        "zshrc: the pwsh file is wired only when pwsh is installed"
    );
    assert!(rc.contains("source \"${${(%):-%x}:h}/pwsh.zsh\""));
    // Before the highlighting plugin, which reads ZSH_HIGHLIGHT_HIGHLIGHTERS when it loads.
    assert!(rc.find("pwsh.zsh").unwrap() < rc.find("zsh-syntax-highlighting.zsh").unwrap());
    // After compinit: the file calls compdef.
    assert!(rc.find("compinit -C").unwrap() < rc.find("pwsh.zsh").unwrap());
    let text = read(ZSH_FILE);
    for gone in ["ps()", "alias ps=", "function ps "] {
        assert!(!text.contains(gone), "pwsh.zsh must not take `ps`: {gone}");
    }
    for want in [
        "pw() {",
        "alias -s ps1=_wolos_ps1",
        "compdef _wolos_pw pw && compdef _wolos_pwsh pwsh",
        "_zsh_highlight_highlighter_pwsh_paint() {",
        "ZSH_HIGHLIGHT_HIGHLIGHTERS+=(pwsh)",
        "-NoLogo -NoProfile -NonInteractive -File \"$_wolos_pw_share/pw.ps1\"",
    ] {
        assert!(text.contains(want), "pwsh.zsh lacks {want}");
    }
    for f in [ZSH_FILE, RUNNER, LISTER] {
        assert!(!read(f).contains('\r'), "{f}: CR");
    }
    for f in [RUNNER, LISTER] {
        assert!(
            read(f).starts_with("# SPDX-License-Identifier: GPL-2.0-or-later\n# WolOS: "),
            "{f}: header"
        );
    }
}

/// Runs `body` in `zsh -f` after sourcing pwsh.zsh, with `home` as HOME, a fresh command list
/// in its cache (so no background job starts) and `bin` first on PATH.
fn zsh_run(home: &Path, body: &str) -> (i32, String, String) {
    let cache = home.join(".cache/wolos/pwsh");
    fs::create_dir_all(&cache).unwrap();
    let tsv = cache.join("commands.tsv");
    if !tsv.exists() {
        fs::write(
            &tsv,
            "*\tcommon\tVerbose Debug ErrorAction\n\
             Sort-Object\tMicrosoft.PowerShell.Utility\tDescending Property Unique\n\
             Get-Process\tMicrosoft.PowerShell.Management\tId Name\n\
             sort\talias of Sort-Object\tDescending Property Unique\n",
        )
        .unwrap();
    }
    let script = format!("emulate zsh; source {}; {body}", repo(ZSH_FILE).display());
    let o = Command::new("zsh")
        .args(["-f", "-c", &script])
        .env_clear()
        .env("HOME", home)
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", home.join("bin").display()),
        )
        .env("WOLOS_PWSH_SHARE", repo(SHARE))
        .env("TERM", "dumb")
        .output()
        .unwrap();
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn pw_hands_the_line_to_the_runner_and_says_whether_stdin_is_a_pipe() {
    let Some(_) = zsh() else {
        eprintln!("zsh not on this host: pw run skipped");
        return;
    };
    let home = sandbox("pw");
    // The fake pwsh prints what it got and exits 5, like a failing line.
    exe(
        &home.join("bin/pwsh"),
        "#!/bin/sh\nprintf 'line=%s input=%s\\n' \"$WOLOS_PW_LINE\" \"$WOLOS_PW_INPUT\"\n\
         for a; do printf '[%s]' \"$a\"; done; echo\ncat >/dev/null\nexit 5\n",
    );
    let (code, out, err) = zsh_run(
        &home,
        "pw 'Sort-Object Length' -x 'a b' </dev/null; print rc=$?; \
         print -l b a | pw '% { $_ }'; print rc=$?; pw; print rc=$?",
    );
    assert_eq!(code, 0, "{err}");
    let runner = repo(RUNNER);
    let want = format!(
        "line=Sort-Object Length input=0\n\
         [-NoLogo][-NoProfile][-NonInteractive][-File][{r}][-x][a b]\nrc=5\n\
         line=% {{ $_ }} input=1\n[-NoLogo][-NoProfile][-NonInteractive][-File][{r}]\nrc=5\nrc=2\n",
        r = runner.display()
    );
    assert_eq!(out, want);
    assert!(err.contains("pw: give one pwsh line"), "{err}");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn ps1_files_typed_as_commands_run_by_shebang_or_through_wol_script() {
    let Some(_) = zsh() else {
        eprintln!("zsh not on this host: suffix run skipped");
        return;
    };
    let home = sandbox("ps1");
    exe(
        &home.join("bin/wol"),
        "#!/bin/sh\nprintf 'wol'; for a; do printf ' [%s]' \"$a\"; done; echo\n",
    );
    exe(&home.join("bin/pwsh"), "#!/bin/sh\nexit 0\n");
    exe(
        &home.join("tidy.ps1"),
        "#!/bin/sh\necho \"shebang run: $*\"\nexit 4\n",
    );
    fs::write(home.join("plain.ps1"), "'no shebang'\n").unwrap();
    // Aliases apply to text read after they exist: eval reads the command lines then.
    let (code, out, err) = zsh_run(
        &home,
        "cd ~; eval './tidy.ps1 a; print rc=$?; tidy.ps1 b; ./plain.ps1 -x \"c d\"; plain.ps1'",
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "shebang run: a\nrc=4\nshebang run: b\n\
         wol [script] [run] [--] [./plain.ps1] [-x] [c d]\nwol [script] [run] [--] [plain.ps1]\n"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn the_persons_pwsh_joins_path_at_the_end_once() {
    let Some(_) = zsh() else {
        eprintln!("zsh not on this host: PATH run skipped");
        return;
    };
    let home = sandbox("path");
    let (_, out, _) = zsh_run(&home, "print -r -- ${path[-1]}");
    assert_ne!(out.trim(), home.join(".local/bin").to_string_lossy());
    exe(&home.join(".local/bin/pwsh"), "#!/bin/sh\nexit 0\n");
    let (code, out, err) = zsh_run(
        &home,
        &format!(
            "source {}; print -r -- ${{path[-1]}} ${{#${{(M)path:#$HOME/.local/bin}}}}",
            repo(ZSH_FILE).display()
        ),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out.trim(),
        format!("{} 1", home.join(".local/bin").display())
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn the_command_list_is_read_once_and_case_insensitively() {
    let Some(_) = zsh() else {
        eprintln!("zsh not on this host: list run skipped");
        return;
    };
    let home = sandbox("list");
    let (code, out, err) = zsh_run(
        &home,
        "_wolos_pw_load && print -r -- \"${_wolos_pw_name[sort-object]}|${_wolos_pw_params[Sort-Object]}|\
         ${_wolos_pw_common}|${_wolos_pw_what[sort]}|${+_wolos_pw_what[(e)*]}\"",
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out.trim(),
        "Sort-Object|Descending Property Unique|Verbose Debug ErrorAction|alias of Sort-Object|0"
    );
    // No list yet: the loader says so (the completer then shows a message and starts the job).
    let (_, out, _) = zsh_run(
        &home,
        "rm -f ~/.cache/wolos/pwsh/commands.tsv; _wolos_pw_stamp=; _wolos_pw_load; print rc=$?",
    );
    assert!(out.ends_with("rc=1\n"), "{out}");
    let _ = fs::remove_dir_all(&home);
}

/// The highlighter's regions for one line: (start, end, style) with BUFFER offsets.
#[test]
fn pwsh_lines_get_colours_where_they_are_and_nowhere_else() {
    let Some(_) = zsh() else {
        eprintln!("zsh not on this host: colour run skipped");
        return;
    };
    let home = sandbox("hl");
    let line = "ls | pw 'Get-ChildItem -Path d | ? { $_.Length -gt 2 } # big' | head";
    let (code, out, err) = zsh_run(
        &home,
        &format!(
            "BUFFER={}; region_highlight=(); _zsh_highlight_highlighter_pwsh_paint; \
             for r in $region_highlight; do print -r -- \"$r ${{BUFFER[${{${{(s: :)r}}[1]}}+1,${{${{(s: :)r}}[2]}}]}}\"; done",
            shell_quote(line)
        ),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "9 22 fg=green Get-ChildItem\n23 28 fg=cyan -Path\n33 34 fg=green ?\n\
         37 39 fg=magenta,bold $_\n47 50 fg=magenta -gt\n55 60 fg=8 # big\n"
    );
    // pwsh -c and -Command: the same; other commands' quoted words: nothing.
    let (_, out, _) = zsh_run(
        &home,
        "for BUFFER in \"pwsh -NoProfile -c 'Get-Date'\" \"echo 'Get-Date'\" \"pwsh x.ps1 Get-Date\"; do \
         region_highlight=(); _zsh_highlight_highlighter_pwsh_paint; print -r -- \"${#region_highlight}\"; done",
    );
    assert_eq!(out, "1\n0\n0\n");
    let _ = fs::remove_dir_all(&home);
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// pw.ps1 in the real pwsh: lines stream into the first command, objects come out as plain text,
/// the status is the line's, a broken line is 2, and the setup's pipe mode is loaded.
#[test]
fn runner_pipes_both_ways_and_keeps_the_status() {
    let Some(pwsh) = pwsh() else {
        eprintln!("pwsh not on this host: runner run skipped");
        return;
    };
    let home = sandbox("runner");
    let run = |line: &str, input: Option<&str>, args: &[&str]| {
        use std::io::Write;
        let mut c = Command::new(&pwsh);
        c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
            .arg(repo(RUNNER))
            .args(args)
            .env_clear()
            .env("HOME", &home)
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "xterm-256color")
            .env("WOLOS_PW_LINE", line)
            .env("WOLOS_PW_INPUT", if input.is_some() { "1" } else { "0" })
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut ch = c.spawn().unwrap();
        let mut stdin = ch.stdin.take().unwrap();
        if let Some(t) = input {
            stdin.write_all(t.as_bytes()).unwrap();
        }
        drop(stdin);
        let o = ch.wait_with_output().unwrap();
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let (code, out, err) = run("Sort-Object Length", Some("ccc\na\nbb\n"), &[]);
    assert_eq!((code, out.as_str()), (0, "a\nbb\nccc\n"), "{err}");
    // Objects: a table without colour codes (stdout is a pipe), rendered before the exit.
    let (code, out, _) = run(
        "[pscustomobject]@{ Name = 'x'; Size = 3 } | Select-Object Name, Size",
        None,
        &[],
    );
    assert_eq!(code, 0);
    assert!(
        out.contains("Name Size") && out.contains("x       3"),
        "{out}"
    );
    assert!(!out.contains('\u{1b}'), "colour codes in a pipe: {out:?}");
    assert_eq!(run("exit 7", None, &[]).0, 7);
    assert_eq!(run("Get-Item /nowhere/at/all", None, &[]).0, 1);
    assert_eq!(run("/bin/sh -c 'exit 9'", None, &[]).0, 9);
    assert_eq!(run("/bin/sh -c 'exit 9'; 'then fine'", None, &[]).0, 0);
    let (code, out, _) = run(
        "\"$($args.Count) $($args -join '|')\"",
        None,
        &["a b", "-x"],
    );
    assert_eq!((code, out.as_str()), (0, "2 a b|-x\n"));
    let (code, _, err) = run("Get-Date |", None, &[]);
    assert_eq!(code, 2);
    assert!(err.starts_with("pw: line 1, column 11: "), "{err}");
    let (code, _, err) = run("'x'", Some("a\n"), &[]);
    assert_eq!(code, 2);
    assert!(err.contains("stdin is a pipe"), "{err}");
    // The line sees no runner variables; the setup's pipe part is there, its prompt part is not.
    let (_, out, _) = run(
        "\"[$env:WOLOS_PW_LINE][$env:WOLOS_PWSH_PIPE]\"; [bool](Get-Command ll -ErrorAction Ignore); \
         [bool](Get-Module PSReadLine); (Get-Variable -Name wolosPipe, wolosTools -ErrorAction Ignore).Count",
        None,
        &[],
    );
    assert_eq!(out, "[][]\nTrue\nFalse\n0\n", "setup {SETUP}");
    let _ = fs::remove_dir_all(&home);
}

/// commands.ps1 in the real pwsh: a `*` line with the common parameters, one line per command
/// with its parameters, written in one move.
#[test]
fn the_lister_writes_commands_with_their_parameters() {
    let Some(pwsh) = pwsh() else {
        eprintln!("pwsh not on this host: lister run skipped");
        return;
    };
    let d = sandbox("lister");
    let out = d.join("cache/commands.tsv");
    let o = Command::new(&pwsh)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(repo(LISTER))
        .arg(&out)
        .env_clear()
        .env("HOME", &d)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = fs::read_to_string(&out).unwrap();
    let first = text.lines().next().unwrap();
    assert!(
        first.starts_with("*\tcommon\t") && first.contains("ErrorAction"),
        "{first}"
    );
    let row = |name: &str| {
        text.lines()
            .find(|l| l.split('\t').next() == Some(name))
            .unwrap_or_else(|| panic!("{name} missing"))
            .split('\t')
            .map(String::from)
            .collect::<Vec<_>>()
    };
    let sort = row("Sort-Object");
    assert!(sort[2].split(' ').any(|p| p == "Descending"), "{sort:?}");
    assert!(
        !sort[2].split(' ').any(|p| p == "ErrorAction"),
        "common ones listed once"
    );
    assert_eq!(row("%")[1], "alias of ForEach-Object");
    assert!(
        fs::read_dir(d.join("cache")).unwrap().count() == 1,
        "no temporary file left"
    );
    let _ = fs::remove_dir_all(&d);
}
