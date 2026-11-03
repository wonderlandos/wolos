// SPDX-License-Identifier: GPL-2.0-or-later
//! pwsh next to zsh (console module, ADR 0032): the wopkg recipe stages it with a launcher that
//! turns off telemetry and the update check, a `$PSHOME` profile that loads the WolOS setup and a
//! launcher entry; the setup gives pwsh the zsh prompt, history, keys and tools, and stays out of
//! scripts. The recipe's steps run with a fake tarball; the setup runs in the real pwsh when one is
//! on PATH (like the zsh tests: skipped without it).
#[path = "support/siblings.rs"]
mod siblings;

use std::{fs, os::unix::fs::PermissionsExt, path::Path, path::PathBuf, process::Command};

const RECIPE: &str = "wopkg/recipes/powershell/recipe.toml";
const SETUP: &str = "desktop/overlay/usr/share/wolos/pwsh/profile.ps1";
/// Where the recipe's `$PSHOME` profile looks for [`SETUP`] on the installed system.
const SETUP_ON_DISK: &str = "/usr/share/wolos/pwsh/profile.ps1";

fn repo(p: &str) -> PathBuf {
    siblings::root().join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// Fresh temp dir with executable fakes `(name, body)` in `bin/`.
fn sandbox(tag: &str, fakes: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-pwsh-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    for (name, body) in fakes {
        let p = d.join("bin").join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    d
}

/// The `run = [...]` strings of one recipe table, TOML basic-string escapes decoded.
fn steps(recipe: &str, table: &str) -> Vec<String> {
    let body = recipe
        .split(&format!("\n[{table}]\n"))
        .nth(1)
        .unwrap_or_else(|| panic!("no [{table}]"));
    let start = body.find("run = [").expect("run = [") + "run = [".len();
    let mut out = Vec::new();
    let (mut cur, mut inside, mut esc) = (String::new(), false, false);
    for c in body[start..].chars() {
        match (inside, esc, c) {
            (true, true, _) => {
                cur.push(c);
                esc = false;
            }
            (true, false, '\\') => esc = true,
            (true, false, '"') => {
                out.push(std::mem::take(&mut cur));
                inside = false;
            }
            (true, false, _) => cur.push(c),
            (false, _, '"') => inside = true,
            (false, _, ']') => break,
            _ => {}
        }
    }
    out
}

/// The real pwsh on PATH, if any, as an absolute path (the runs below clear PATH). CI hosts have
/// none; the forge test run puts one there.
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
fn recipe_stages_launcher_profile_hook_and_entry() {
    let text = read(RECIPE);
    // Pinned: one https source with its sha256, MIT, user-side.
    assert!(text.contains("\nlicense = \"MIT\"\n"));
    assert!(
        text.contains(
            "sha256 = \"1da6d6d6035d651f20e70ddee7c4c15ac6ab8b9fb40ee92240d91263f28a884b\""
        )
    );
    let d = sandbox("recipe", &[]);
    let (src, out) = (d.join("src"), d.join("out"));
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&out).unwrap();
    // The tarball's pwsh: prints what the launcher handed it.
    fs::write(
        src.join("pwsh"),
        "#!/bin/sh\necho \"args=$* t=$POWERSHELL_TELEMETRY_OPTOUT u=$POWERSHELL_UPDATECHECK d=$DOTNET_CLI_TELEMETRY_OPTOUT\"\n",
    )
    .unwrap();
    // TARGET_PREFIX = the staging dir, so the launcher can run here.
    for step in steps(&text, "build").iter().chain(&steps(&text, "install")) {
        let st = Command::new("sh")
            .args(["-eu", "-c", step])
            .current_dir(&src)
            .env("PREFIX", &out)
            .env("TARGET_PREFIX", &out)
            .status()
            .unwrap();
        assert!(st.success(), "step failed: {step}");
    }
    let launcher = fs::read_to_string(out.join("bin/pwsh")).unwrap();
    assert!(launcher.starts_with("#!/bin/sh\n"), "{launcher}");
    let o = Command::new(out.join("bin/pwsh"))
        .args(["-a", "b c"])
        .env_remove("POWERSHELL_TELEMETRY_OPTOUT")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).trim(),
        "args=-a b c t=1 u=Off d=1",
        "launcher: {launcher}"
    );
    // $PSHOME/profile.ps1 (all users, all hosts) loads the WolOS setup only when it is there.
    let hook = fs::read_to_string(out.join("lib/powershell/profile.ps1")).unwrap();
    assert!(
        hook.contains(&format!(
            "if ([IO.File]::Exists('{SETUP_ON_DISK}')) {{ . '{SETUP_ON_DISK}' }}"
        )),
        "{hook}"
    );
    assert!(repo(SETUP).is_file());
    // The launcher entry: in the user's data dir, so it exists only while pwsh is installed.
    let entry = fs::read_to_string(out.join("share/applications/wolos-pwsh.desktop")).unwrap();
    let exec = format!("Exec=foot -T pwsh {}/bin/pwsh", out.display());
    for l in [
        "[Desktop Entry]",
        "Type=Application",
        "Name=pwsh",
        "Categories=System;TerminalEmulator;",
        exec.as_str(),
    ] {
        assert!(entry.lines().any(|x| x == l), "entry lacks {l}:\n{entry}");
    }
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn setup_matches_zsh_and_stays_out_of_scripts() {
    let text = read(SETUP);
    let zsh = read("desktop/overlay/usr/share/wolos/zsh/zshrc");
    // Same history size as zsh, and the interactive guard (scripts get a plain pwsh).
    assert!(zsh.contains("HISTSIZE=50000"));
    for p in [
        "-MaximumHistoryCount 50000",
        "-PredictionSource History",
        "[Console]::IsInputRedirected",
        "$env:WOLOS_PWSH_FORCE -ne '1'",
        "function global:prompt",
        "& $wolosInit wopkg @('completions', 'pwsh')",
        "wolos/toolkits/pwsh/colors.ps1",
        "$env:SSH_CONNECTION",
    ] {
        assert!(text.contains(p), "profile.ps1 lacks {p}");
    }
    assert_eq!(text.matches('{').count(), text.matches('}').count());
    assert!(!text.contains('\r'));
    let Some(pwsh) = pwsh() else {
        eprintln!("pwsh not on this host: profile run skipped");
        return;
    };
    // It parses.
    let probe = format!(
        "$e = $null; \
         $null = [System.Management.Automation.Language.Parser]::ParseFile('{}', [ref] $null, [ref] $e); \
         $e | ForEach-Object {{ $_.ToString() }}; exit $e.Count",
        repo(SETUP).display()
    );
    let o = Command::new(&pwsh)
        .args(["-NoProfile", "-NonInteractive", "-Command", &probe])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    // A script (-Command, redirected output) skips it; WOLOS_PWSH_FORCE=1 loads it.
    let d = sandbox("guard", &[]);
    let home = d.join("home");
    fs::create_dir_all(&home).unwrap();
    let run = |force: bool| {
        let probe = format!(
            ". '{}'; (Get-PSReadLineOption).MaximumHistoryCount; \
             Test-Path function:global:prompt -PathType Leaf; [bool](Get-Command ll -ErrorAction Ignore); \
             (Get-Variable -Name wolos*, batch, argv, i, t, d, e -ErrorAction Ignore).Count",
            repo(SETUP).display()
        );
        let mut c = Command::new(&pwsh);
        c.args(["-NoProfile", "-NonInteractive", "-Command", &probe])
            .env_clear()
            .env("HOME", &home)
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "foot");
        if force {
            c.env("WOLOS_PWSH_FORCE", "1");
        }
        let o = c.output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    // Nothing of the setup is left behind but its functions (no variables).
    assert!(
        run(false).ends_with("False 0"),
        "script got the setup: {}",
        run(false)
    );
    assert_eq!(run(true), "50000 True True 0");
    let _ = fs::remove_dir_all(&d);
}

/// One forced pwsh run of the setup with fakes on PATH; stdout lines.
fn setup_run(d: &Path, env: &[(&str, &str)], after: &str) -> String {
    let probe = format!(". '{}'; {after}", repo(SETUP).display());
    let mut c = Command::new(pwsh().unwrap());
    c.args(["-NoProfile", "-NonInteractive", "-Command", &probe])
        .current_dir(d.join("repo/sub"))
        .env_clear()
        .env("HOME", d.join("home"))
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("WOLOS_PWSH_FORCE", "1")
        .env("TERM", "foot")
        .env("COLORTERM", "truecolor");
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success() && err.trim().is_empty(), "{err}");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn setup_gives_the_zsh_prompt_keys_completion_and_tools() {
    if pwsh().is_none() {
        eprintln!("pwsh not on this host: profile run skipped");
        return;
    }
    let log = "echo \"$(basename \"$0\") $*\" >> \"$HOME/log\"";
    let d = sandbox(
        "setup",
        &[
            (
                "starship",
                &format!(
                    "#!/bin/sh\n{log}\n[ \"$1\" = init ] && echo \"function global:prompt {{ 'STAR> ' }}\"\n"
                ),
            ),
            (
                "wopkg",
                &format!(
                    "#!/bin/sh\n{log}\ncase $1 in\n__complete) printf 'firefox\\n' ;;\ncompletions) cat \"$HOME/comp.ps1\" ;;\nesac\n"
                ),
            ),
            ("eza", "#!/bin/sh\necho \"eza $*\"\n"),
            ("fzf", "#!/bin/sh\nexit 1\n"),
        ],
    );
    let home = d.join("home");
    fs::create_dir_all(d.join("repo/.git")).unwrap();
    fs::create_dir_all(d.join("repo/sub")).unwrap();
    fs::write(d.join("repo/.git/HEAD"), "ref: refs/heads/topic/pwsh\n").unwrap();
    // The theme's colours (what `wol theme` writes).
    let colors = home.join(".config/wolos/toolkits/pwsh/colors.ps1");
    fs::create_dir_all(colors.parent().unwrap()).unwrap();
    fs::write(
        &colors,
        "$env:WOLOS_PROMPT_ACCENT = '#7c5cff'\n$env:WOLOS_PROMPT_MUTED = '#686575'\n\
         $env:WOLOS_PROMPT_OK = '#167b58'\n$env:WOLOS_PROMPT_ERROR = '#c53763'\n",
    )
    .unwrap();
    // A stand-in for `wopkg completions pwsh` (the real script is tested in wopkg's specs).
    fs::write(
        home.join("comp.ps1"),
        "Register-ArgumentCompleter -Native -CommandName wopkg -ScriptBlock { \
         param($w) [System.Management.Automation.CompletionResult]::new('install') }\n",
    )
    .unwrap();

    // Pure prompt: accent directory (~-relative), muted branch, OK symbol; error after a failure.
    let pure = setup_run(
        &d,
        &[("WOLOS_PROMPT", "pure")],
        "prompt; Get-Item /nonexistent -ErrorAction SilentlyContinue; prompt",
    );
    let accent = "\u{1b}[38;2;124;92;255m";
    let lines: Vec<&str> = pure.lines().collect();
    assert_eq!(lines.len(), 2, "{pure:?}");
    let dir = d.join("repo/sub");
    let shown = dir.display().to_string();
    let parts: Vec<&str> = shown.split('/').filter(|p| !p.is_empty()).collect();
    let want_dir = if parts.len() >= 4 {
        format!(".../{}", parts[parts.len() - 3..].join("/"))
    } else {
        shown.clone()
    };
    assert!(
        lines[0].starts_with(&format!("{accent}{want_dir}\u{1b}[0m")),
        "{:?}",
        lines[0]
    );
    assert!(
        lines[0].contains("\u{1b}[38;2;104;101;117m topic/pwsh\u{1b}[0m"),
        "{:?}",
        lines[0]
    );
    assert!(
        lines[0].ends_with(" \u{1b}[38;2;22;123;88m\u{276f}\u{1b}[0m "),
        "{:?}",
        lines[0]
    );
    assert!(
        lines[1].ends_with(" \u{1b}[38;2;197;55;99m\u{276f}\u{1b}[0m "),
        "after a failure: {:?}",
        lines[1]
    );
    assert!(!pure.contains("@"), "user@host only over SSH: {pure:?}");
    let ssh = setup_run(
        &d,
        &[("WOLOS_PROMPT", "pure"), ("SSH_CONNECTION", "1 2 3 4")],
        "prompt",
    );
    assert!(
        ssh.starts_with("\u{1b}[38;2;104;101;117m") && ssh.contains('@'),
        "{ssh:?}"
    );
    assert!(
        !home.join("log").exists()
            || !fs::read_to_string(home.join("log"))
                .unwrap()
                .contains("starship"),
        "WOLOS_PROMPT=pure starts starship"
    );

    // starship's prompt, its init cached: the second window does not start starship.
    let keys = "prompt; \
        foreach ($k in 'UpArrow', 'DownArrow', 'Tab', 'Shift+Tab', 'Ctrl+r', 'Ctrl+RightArrow', 'Home', 'End') { \
          $h = Get-PSReadLineKeyHandler -Bound | Where-Object Key -eq $k | Select-Object -First 1; \"$k=$($h.Function)\" }; \
        (TabExpansion2 -inputScript 'wopkg ins' -cursorColumn 9).CompletionMatches.CompletionText; \
        ll x; (Get-PSReadLineOption).PredictionSource";
    let first = setup_run(&d, &[], keys);
    let second = setup_run(&d, &[], "prompt");
    assert_eq!(second.trim(), "STAR>");
    let lines: Vec<&str> = first.lines().map(str::trim).collect();
    assert_eq!(
        lines,
        [
            "STAR>",
            "UpArrow=HistorySearchBackward",
            "DownArrow=HistorySearchForward",
            "Tab=MenuComplete",
            "Shift+Tab=TabCompletePrevious",
            "Ctrl+r=WolosHistory",
            "Ctrl+RightArrow=ForwardWord",
            "Home=BeginningOfLine",
            "End=EndOfLine",
            "install",
            "eza -lh --git x",
            "History",
        ],
        "{first}"
    );
    let log = fs::read_to_string(home.join("log")).unwrap();
    assert_eq!(
        log.matches("starship init powershell --print-full-init")
            .count(),
        1,
        "starship init not cached:\n{log}"
    );
    assert_eq!(log.matches("wopkg completions pwsh").count(), 1, "{log}");
    assert!(home.join(".cache/wolos/pwsh").is_dir());
    // Only the cache is written; the user's own profile is never created.
    assert!(!home.join(".config/powershell").exists());
    let _ = fs::remove_dir_all(&d);
}

/// `pwsh/check.ps1`: pwsh's parse-only check (it has no `-n`), for the script kinds' syntax step.
/// It reads the file with the parser and never runs it.
#[test]
fn checker_parses_scripts_without_running_them() {
    let check = "desktop/overlay/usr/share/wolos/pwsh/check.ps1";
    let text = read(check);
    assert!(text.contains("Language.Parser]::ParseFile("));
    assert!(!text.contains('\r'));
    let Some(pwsh) = pwsh() else {
        eprintln!("pwsh not on this host: checker run skipped");
        return;
    };
    let d = sandbox("check", &[]);
    let ran = d.join("ran");
    let good = d.join("good.ps1");
    fs::write(
        &good,
        format!("Set-Content -LiteralPath '{}' -Value x\n", ran.display()),
    )
    .unwrap();
    fs::write(d.join("bad.ps1"), "if ($x {\n").unwrap();
    let run = |f: &str| {
        let o = Command::new(&pwsh)
            .args(["-NoProfile", "-NonInteractive", "-File"])
            .arg(repo(check))
            .arg(f)
            .current_dir(&d)
            .output()
            .unwrap();
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    assert_eq!(run("good.ps1"), (Some(0), String::new()));
    assert!(!ran.exists(), "the checker ran the script");
    let (code, err) = run("bad.ps1");
    assert_eq!(code, Some(1));
    assert!(err.starts_with("bad.ps1:1:"), "{err}");
    assert_eq!(run("missing.ps1").0, Some(2));
    let _ = fs::remove_dir_all(&d);
}

/// `/usr/lib/wolos/pwsh`: the fixed path `wol script` runs .ps1 files with. It starts the
/// person's own pwsh (`~/.local/bin/pwsh`, the recipe's launcher) with the arguments untouched, and
/// without it says where pwsh comes from and exits 127.
#[test]
fn system_shim_starts_the_persons_pwsh_or_says_where_it_comes_from() {
    let shim = "files/usr/lib/wolos/pwsh";
    let st = Command::new("sh")
        .arg("-n")
        .arg(repo(shim))
        .status()
        .unwrap();
    assert!(st.success(), "sh -n {shim}");
    assert!(read(shim).starts_with("#!/bin/sh\n# SPDX-License-Identifier: GPL-2.0-or-later\n"));
    let d = sandbox("shim", &[]);
    let home = d.join("home");
    let run = |home: Option<&Path>| {
        let mut c = Command::new("sh");
        c.arg(repo(shim))
            .args(["-File", "a b.ps1", "-x"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin");
        if let Some(h) = home {
            c.env("HOME", h);
        }
        let o = c.output().unwrap();
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let hint = "pwsh comes with the Console module: run wol modules enable console.\n";
    for h in [None, Some(home.as_path())] {
        assert_eq!(run(h), (Some(127), String::new(), hint.to_string()));
    }
    let bin = home.join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(
        bin.join("pwsh"),
        "#!/bin/sh\nfor a; do echo \"[$a]\"; done\n",
    )
    .unwrap();
    // Not executable yet: still the hint.
    assert_eq!(run(Some(&home)).0, Some(127));
    fs::set_permissions(bin.join("pwsh"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        run(Some(&home)),
        (
            Some(0),
            "[-File]\n[a b.ps1]\n[-x]\n".to_string(),
            String::new()
        )
    );
    let _ = fs::remove_dir_all(&d);
}

/// `/usr/bin/wol-term-shell`: foot, ghostty and kitty start it; it opens the shell chosen in
/// Settings > General (`wolos/terminal-shell`) with SHELL set to it, and zsh with one sentence for
/// an unknown word or a shell that is not installed. Its table is `wol_core::terminal_shell`'s.
#[test]
fn terminals_open_the_shell_chosen_in_settings() {
    use wol_core::terminal_shell as ts;
    let launcher = "desktop/overlay/usr/bin/wol-term-shell";
    let text = read(launcher);
    assert_eq!(format!("/usr/bin/{}", "wol-term-shell"), ts::LAUNCHER);
    let st = Command::new("sh")
        .arg("-n")
        .arg(repo(launcher))
        .status()
        .unwrap();
    assert!(st.success(), "sh -n {launcher}");
    for prog in ts::SHELLS {
        let id = ts::id(prog);
        let arm = match prog.strip_prefix("~/") {
            Some(rest) => format!("{id}) p=$HOME/{rest} ;;"),
            None => format!("{id}) p=$root{prog} ;;"),
        };
        assert!(text.contains(&arm), "{launcher} lacks `{arm}`");
    }
    assert!(text.contains(&format!(
        "/wolos/{}\"",
        ts::FILE.trim_start_matches("wolos/")
    )));
    // Every terminal starts it (foot.ini is the default theme's render).
    let foot = read("desktop/overlay/usr/share/wolos/foot/foot.ini");
    assert!(
        foot.lines().any(|l| l == format!("shell={}", ts::LAUNCHER)),
        "foot.ini"
    );
    let ghostty = read("desktop/overlay/usr/share/wolos/ghostty/config");
    assert!(
        ghostty
            .lines()
            .any(|l| l == format!("command = {}", ts::LAUNCHER)),
        "ghostty"
    );
    let kitty = read("desktop/overlay/usr/share/wolos/kitty/kitty.conf");
    assert!(
        kitty
            .lines()
            .any(|l| l == format!("shell {}", ts::LAUNCHER)),
        "kitty"
    );

    // Fake shells print who they are, SHELL and their arguments.
    let d = sandbox("termshell", &[]);
    let (root, home) = (d.join("root"), d.join("home"));
    let say = "#!/bin/sh\necho \"$(basename \"$0\") SHELL=$SHELL $*\"\n";
    for p in ["usr/bin/zsh", "usr/bin/fish"] {
        let f = root.join(p);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, say).unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let pwsh = home.join(".local/bin/pwsh");
    fs::create_dir_all(pwsh.parent().unwrap()).unwrap();
    fs::write(&pwsh, say).unwrap();
    fs::set_permissions(&pwsh, fs::Permissions::from_mode(0o755)).unwrap();
    let choice = home.join(".config").join(ts::FILE);
    fs::create_dir_all(choice.parent().unwrap()).unwrap();
    let run = |word: Option<&str>| {
        match word {
            Some(w) => fs::write(&choice, w).unwrap(),
            None => {
                let _ = fs::remove_file(&choice);
            }
        }
        let o = Command::new("sh")
            .arg(repo(launcher))
            .arg("-l")
            .env_clear()
            .env("HOME", &home)
            .env("PATH", "/usr/bin:/bin")
            .env("WOLOS_TERM_SHELL_ROOT", &root)
            .output()
            .unwrap();
        assert!(o.status.success());
        (
            String::from_utf8_lossy(&o.stdout).trim().to_string(),
            String::from_utf8_lossy(&o.stderr).trim().to_string(),
        )
    };
    let zsh = format!("zsh SHELL={}/usr/bin/zsh -l", root.display());
    assert_eq!(run(None), (zsh.clone(), String::new()), "no file: zsh");
    assert_eq!(run(Some("zsh\n")), (zsh.clone(), String::new()));
    assert_eq!(
        run(Some("pwsh\n")),
        (format!("pwsh SHELL={} -l", pwsh.display()), String::new())
    );
    // No trailing newline is the same word.
    assert_eq!(
        run(Some("fish")).0,
        format!("fish SHELL={}/usr/bin/fish -l", root.display())
    );
    for (word, shown) in [("nu\n", "nu"), ("tcsh\n", "tcsh")] {
        assert_eq!(
            run(Some(word)),
            (
                zsh.clone(),
                format!(
                    "The shell chosen in Settings ({shown}) is not installed, so this window uses zsh."
                )
            )
        );
    }
    let _ = fs::remove_dir_all(&d);
}
