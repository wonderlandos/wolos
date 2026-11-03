// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol script` (ADR 0041, docs/modules/scripting.md): a Ruby or pwsh script is checked and run
//! exactly like a shell script. Every parity test walks the same cases for `.sh`, `.rb` and `.ps1`: the argv
//! (interpreter + file, never a shell string), the clean environment, the helper name, the audit
//! record, the ssh path (file on stdin) and the refusals. The last tests run the real `wol` binary
//! on real scripts and skip with a message when the interpreter is missing.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use controller::script::{KINDS, Kind};
use std::path::{Path, PathBuf};

fn temp_home(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-script-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

/// (kind id, file name, text, interpreter) for one script of each kind.
const SCRIPTS: &[(&str, &str, &str, &str)] = &[
    ("sh", "tidy.sh", "#!/bin/sh\necho hi\n", "/bin/sh"),
    (
        "bash",
        "tidy.bash",
        "#!/usr/bin/env bash\necho hi\n",
        "/bin/bash",
    ),
    (
        "ruby",
        "tidy.rb",
        "#!/usr/bin/ruby\nputs 'hi'\n",
        "/usr/bin/ruby",
    ),
    (
        "ruby",
        "tidy",
        "#!/usr/bin/env ruby\nputs 'hi'\n",
        "/usr/bin/ruby",
    ),
    (
        "ps1",
        "tidy.ps1",
        "'hi'\n",
        "/usr/lib/wolos/pwsh",
    ),
    (
        "ps1",
        "tidy",
        "#!/usr/bin/env pwsh\n'hi'\n",
        "/usr/lib/wolos/pwsh",
    ),
];

fn kind(id: &str) -> &'static Kind {
    KINDS.iter().find(|k| k.id == id).unwrap()
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn env_with(tag: &str, file: &str, text: &str) -> (FakeEnv, String) {
    let home = temp_home(tag);
    let path = home.join(file).to_string_lossy().into_owned();
    let env = FakeEnv {
        files: vec![(PathBuf::from(&path), text.into())],
        vars: vec![
            ("HOME".into(), home.to_string_lossy().into_owned()),
            ("XDG_RUNTIME_DIR".into(), "/run/user/1000".into()),
            ("LANG".into(), "pt_BR.UTF-8".into()),
            ("GITHUB_TOKEN".into(), "ghp_secret".into()),
            ("PATH".into(), "/home/x/.local/bin:/usr/bin".into()),
        ],
        home,
        ..Default::default()
    };
    (env, path)
}

fn audit_log(env: &FakeEnv) -> Vec<serde_json::Value> {
    std::fs::read_to_string(env.home.join("state/wolos/actions.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn every_kind_runs_as_an_argv_with_a_clean_environment() {
    for (i, (kind, file, text, prog)) in SCRIPTS.iter().enumerate() {
        let (mut e, path) = env_with(&format!("run{i}"), file, text);
        e.exec_codes = vec![3];
        let line = format!("script run {path} -- a --b");
        assert_eq!(run(&line, &mut e), 3, "{kind}: the script's exit code");
        assert_eq!(e.execs.len(), 1, "{kind}");
        let argv = &e.execs[0];
        assert_eq!(&argv[..2], ["/usr/bin/env", "-i"], "{kind}: {argv:?}");
        let at = argv.iter().position(|a| a == prog).expect(prog);
        let mut want = strings(self::kind(kind).run_args);
        want.extend(strings(&[path.as_str(), "a", "--b"]));
        assert_eq!(&argv[at + 1..], want, "{kind}");
        let vars = &argv[2..at];
        assert!(vars.contains(&"WOL_HELPER=tidy".to_string()), "{vars:?}");
        assert!(vars.contains(&"PATH=/usr/local/bin:/usr/bin".to_string()));
        assert!(vars.contains(&"XDG_RUNTIME_DIR=/run/user/1000".to_string()));
        assert!(vars.contains(&"LANG=pt_BR.UTF-8".to_string()));
        assert!(
            !vars
                .iter()
                .any(|v| v.contains("GITHUB_TOKEN") || v.contains(".local/bin")),
            "{kind}: tokens and the caller's PATH stay out: {vars:?}"
        );
        let log = audit_log(&e);
        assert_eq!(log.len(), 1, "{kind}: one record per run");
        assert_eq!(log[0]["helper"], "script:tidy");
        assert_eq!(log[0]["tool"], "script run");
        assert_eq!(log[0]["via"], "cli");
        assert_eq!(log[0]["result"], "failed");
        assert_eq!(log[0]["detail"], "exit 3");
        assert_eq!(
            log[0]["args"],
            serde_json::json!([path]),
            "{kind}: no script args"
        );
    }
}

#[test]
fn every_kind_runs_on_another_computer_from_stdin() {
    for (i, (kind, file, text, prog)) in SCRIPTS.iter().enumerate() {
        let (mut e, path) = env_with(&format!("on{i}"), file, text);
        assert_eq!(
            run(&format!("script on klee@box.lan {path} x"), &mut e),
            0,
            "{kind}: {}",
            e.err
        );
        let argv = &e.execs[0];
        assert_eq!(
            &argv[..7],
            [
                "ssh",
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=20",
                "--"
            ]
        );
        assert_eq!(argv[7], "klee@box.lan");
        let cmd = &argv[8];
        assert!(
            cmd.starts_with("exec /usr/bin/env -i HOME=\"$HOME\""),
            "{cmd}"
        );
        assert!(cmd.contains(&format!("'{prog}'")), "{kind}: {cmd}");
        assert!(
            cmd.contains("'WOL_HELPER=tidy'") && cmd.ends_with(" 'x'"),
            "{cmd}"
        );
        assert!(
            !cmd.contains(&path),
            "{kind}: the file goes on stdin, not by name"
        );
        assert_eq!(e.inputs, vec![text.to_string()], "{kind}");
        let log = audit_log(&e);
        assert_eq!(log[0]["tool"], "script on");
        assert_eq!(log[0]["via"], "ssh");
        assert_eq!(log[0]["result"], "ok");
        assert_eq!(log[0]["args"], serde_json::json!(["klee@box.lan", path]));
    }
}

#[test]
fn refusals_are_the_same_for_every_kind() {
    for (i, (kind, file, text, _)) in SCRIPTS.iter().enumerate() {
        let (mut e, path) = env_with(&format!("no{i}"), file, text);
        e.root = true;
        assert_eq!(run(&format!("script run {path}"), &mut e), 77, "{kind}");
        assert!(e.err.contains("never as root"), "{}", e.err);
        e.root = false;
        assert_eq!(
            run(&format!("script on -oProxyCommand=x {path}"), &mut e),
            64,
            "{kind}"
        );
        assert!(e.err.contains("not a host name"), "{}", e.err);
        assert!(
            e.execs.is_empty() && e.inputs.is_empty(),
            "{kind}: nothing ran"
        );
        assert!(audit_log(&e).is_empty(), "{kind}: nothing to record");
    }
    let (mut e, path) = env_with("py", "x.py", "#!/usr/bin/python3\nprint(1)\n");
    assert_eq!(run(&format!("script run {path}"), &mut e), 69);
    assert!(e.err.contains("is a python script"), "{}", e.err);
    let (mut e, _) = env_with("missing", "x.rb", "");
    assert_eq!(run("script run /nowhere/x.rb", &mut e), 66);
    assert!(e.execs.is_empty());
}

#[test]
fn a_file_named_like_an_option_stays_a_file() {
    let (mut e, _) = env_with("dash", "x.rb", "");
    e.files = vec![(PathBuf::from("-e.rb"), "puts 1\n".into())];
    assert_eq!(run("script run -- -e.rb", &mut e), 0, "{}", e.err);
    assert_eq!(e.execs[0].last().unwrap(), "./-e.rb");
}

#[test]
fn check_parses_without_running_then_lints_when_installed() {
    for (i, (kind, file, text, prog)) in SCRIPTS.iter().enumerate() {
        let (mut e, path) = env_with(&format!("chk{i}"), file, text);
        assert_eq!(run(&format!("script check {path}"), &mut e), 0, "{kind}");
        assert_eq!(e.execs.len(), 1, "{kind}: no linter installed, syntax only");
        let mut want = vec![prog.to_string()];
        want.extend(strings(self::kind(kind).syntax));
        want.push(path.clone());
        assert_eq!(e.execs[0], want);
        assert!(e.out.contains("parses"), "{}", e.out);
        assert!(audit_log(&e).is_empty(), "a check is a read");

        // pwsh's launcher says "not installed" with 127: a missing tool, not a parse error
        let (mut e, path) = env_with(&format!("gone{i}"), file, text);
        e.exec_codes = vec![127];
        assert_eq!(run(&format!("script check {path}"), &mut e), 65);
        assert!(e.out.contains("is not installed"), "{kind}: {}", e.out);

        // the linter is installed: it runs after a clean parse
        if self::kind(kind).lint.is_none() {
            continue;
        }
        let (mut e, path) = env_with(&format!("lint{i}"), file, text);
        e.captured = Some("1.0\n".into());
        e.exec_codes = vec![0, 1];
        assert_eq!(run(&format!("script check {path} --json"), &mut e), 65);
        let lint = &e.execs[1];
        let tool = if *prog == "/usr/bin/ruby" {
            assert!(lint.contains(&"--only".to_string()), "{lint:?}");
            assert!(lint.contains(&"--force-default-config".to_string()));
            "rubocop"
        } else {
            "shellcheck"
        };
        assert_eq!(lint[0], tool);
        assert_eq!(lint.last().unwrap(), &path);
        let v: serde_json::Value = serde_json::from_str(&e.out).unwrap();
        assert_eq!(v["kind"], *kind);
        assert_eq!(v["ok"], false);
        assert_eq!(v["lint"], tool);
        assert!(v["problems"][0].as_str().unwrap().contains(tool));

    }
    // a parse error stops before the linter (every kind)
    for (i, (kind, file, text, _)) in SCRIPTS.iter().enumerate() {
        let (mut e, path) = env_with(&format!("bad{i}"), file, text);
        e.captured = Some("1.0\n".into());
        e.exec_codes = vec![1];
        assert_eq!(run(&format!("script check {path}"), &mut e), 65);
        assert_eq!(e.execs.len(), 1);
        assert!(
            e.out.contains(&format!("does not parse as {kind}")),
            "{}",
            e.out
        );
    }
}

#[test]
fn kinds_list_names_ruby_and_ps1_beside_sh() {
    let mut e = FakeEnv::default();
    assert_eq!(run("script --json", &mut e), 0);
    let v: serde_json::Value = serde_json::from_str(&e.out).unwrap();
    let ids: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["sh", "bash", "ruby", "ps1"]);
    assert_eq!(v[2]["extensions"], serde_json::json!([".rb"]));
    assert_eq!(v[3]["extensions"], serde_json::json!([".ps1"]));
    assert_eq!(v[3]["program"], "/usr/lib/wolos/pwsh");
    assert_eq!(v[3]["lint"], serde_json::Value::Null);
}

#[test]
fn helpers_may_check_but_not_run() {
    use controller::catalog::rule_of;
    let p = |l: &str| parse(&l.split_whitespace().collect::<Vec<_>>()).unwrap();
    assert!(rule_of(&p("script check x.rb")).is_some());
    assert!(rule_of(&p("script")).is_some());
    assert!(rule_of(&p("script run x.rb")).is_none());
    assert!(rule_of(&p("script on box x.rb")).is_none());
    assert!(
        controller::catalog::find_tool("script_run").is_none(),
        "wol mcp never offers running code"
    );
}

// ---------------------------------------------------------------- the real binary

fn have(prog: &str) -> bool {
    Path::new(prog).exists()
}

/// The real pwsh on PATH (the forge and VM runs put one there; CI hosts have none).
fn pwsh_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("pwsh"))
        .find(|p| p.is_file())
}

/// Whether `prog` can run a script with `home` as HOME. The pwsh shim starts the person's pwsh
/// (~/.local/bin/pwsh): `home` gets a link to the real one, or the case is skipped.
fn ready(prog: &str, home: &Path) -> bool {
    if !have(prog) {
        return false;
    }
    if prog != "/usr/lib/wolos/pwsh" {
        return true;
    }
    let Some(real) = pwsh_on_path() else {
        return false;
    };
    let bin = home.join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let _ = std::fs::remove_file(bin.join("pwsh"));
    std::os::unix::fs::symlink(real, bin.join("pwsh")).is_ok()
}

/// Runs the built `wol` with only `vars` in its environment.
fn wol(args: &[&str], vars: &[(&str, &Path)]) -> std::process::Output {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_wol"));
    c.args(args).env_clear().env("GITHUB_TOKEN", "ghp_secret");
    for (k, v) in vars {
        c.env(k, v);
    }
    c.output().unwrap()
}

/// What a script sees, and what its own `wol` call meets: the helper gate (no session here, so
/// a change is refused with the session message) and one audit record under its name.
#[test]
fn real_scripts_get_the_same_environment_and_the_same_gate() {
    let wol_bin = env!("CARGO_BIN_EXE_wol");
    let cases: &[(&str, &str, String)] = &[
        (
            "/bin/sh",
            "probe.sh",
            "#!/bin/sh\nprintf 'helper=%s token=%s\\n' \"$WOL_HELPER\" \"${GITHUB_TOKEN:-none}\"\n\
             \"$1\" focus on\nprintf 'code=%s\\n' \"$?\"\n"
                .to_string(),
        ),
        (
            "/usr/bin/ruby",
            "probe.rb",
            "#!/usr/bin/ruby\n# frozen_string_literal: true\n\
             puts \"helper=#{ENV['WOL_HELPER']} token=#{ENV.fetch('GITHUB_TOKEN', 'none')}\"\n\
             system(ARGV[0], 'focus', 'on')\nputs \"code=#{$?.exitstatus}\"\n"
                .to_string(),
        ),
        (
            "/usr/lib/wolos/pwsh",
            "probe.ps1",
            "#!/usr/bin/env pwsh\n\
             \"helper=$env:WOL_HELPER token=$(if ($env:GITHUB_TOKEN) { 'set' } else { 'none' })\"\n\
             & $args[0] focus on\n\"code=$LASTEXITCODE\"\n"
                .to_string(),
        ),
    ];
    for (prog, name, text) in cases {
        let home = temp_home(&format!("real-{name}"));
        if !ready(prog, &home) {
            eprintln!("{prog} not installed; {name} skipped");
            continue;
        }
        let file = home.join(name);
        std::fs::write(&file, text).unwrap();
        let state = home.join("state");
        let o = wol(
            &["script", "run", file.to_str().unwrap(), wol_bin],
            &[("HOME", &home), ("XDG_STATE_HOME", &state)],
        );
        let out = String::from_utf8_lossy(&o.stdout);
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(o.status.success(), "{name}: {out} {err}");
        assert!(out.contains("helper=probe token=none"), "{name}: {out}");
        assert!(
            out.contains("code=69"),
            "{name}: change refused without a session: {out}"
        );
        assert!(err.contains("No WolOS session"), "{name}: {err}");
        let log = std::fs::read_to_string(state.join("wolos/actions.jsonl")).unwrap();
        let recs: Vec<serde_json::Value> = log
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(recs.len(), 2, "{name}: {log}");
        assert_eq!(recs[0]["helper"], "script:probe");
        assert_eq!(recs[0]["tool"], "focus on");
        assert_eq!(recs[1]["tool"], "script run");
        assert_eq!(recs[1]["result"], "ok");
    }
}

#[test]
fn real_check_tells_a_good_script_from_a_broken_one() {
    for (prog, good, bad) in [
        ("/bin/sh", "#!/bin/sh\necho ok\n", "#!/bin/sh\nif then fi\n"),
        (
            "/usr/bin/ruby",
            "#!/usr/bin/ruby\n# frozen_string_literal: true\nputs 'ok'\n",
            "#!/usr/bin/ruby\ndef x(\n",
        ),
        (
            "/usr/lib/wolos/pwsh",
            "#!/usr/bin/env pwsh\nparam($Name)\n\"ok $Name\"\n",
            "#!/usr/bin/env pwsh\nif ($x {\n",
        ),
    ] {
        let home = temp_home(&format!("realchk-{}", prog.replace('/', "")));
        if !ready(prog, &home) {
            eprintln!("{prog} not installed; check skipped");
            continue;
        }
        for (n, t, ok) in [("good", good, true), ("bad", bad, false)] {
            let f = home.join(n);
            std::fs::write(&f, t).unwrap();
            let o = wol(
                &["script", "check", f.to_str().unwrap(), "--json"],
                &[("HOME", &home)],
            );
            let v: serde_json::Value = serde_json::from_slice(&o.stdout)
                .unwrap_or_else(|e| panic!("{prog} {n}: {e}: {:?}", o));
            assert_eq!(v["ok"], ok, "{prog} {n}: {v}");
            assert_eq!(o.status.success(), ok);
        }
    }
}

/// `wol script on` end to end with a stand-in for ssh that does what sshd does with the command
/// line: hands it to the other side's shell, stdin attached. The script arrives on stdin, gets the
/// clean environment and the helper name there, and its arguments arrive unchanged, quotes and all.
#[test]
fn real_scripts_run_on_another_computer_from_stdin() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "/bin/sh",
            "far.sh",
            "#!/bin/sh\nprintf 'helper=%s token=%s arg=%s\\n' \"$WOL_HELPER\" \"${GITHUB_TOKEN:-none}\" \"$1\"\n",
        ),
        (
            "/usr/bin/ruby",
            "far.rb",
            "#!/usr/bin/ruby\n# frozen_string_literal: true\n\
             puts \"helper=#{ENV['WOL_HELPER']} token=#{ENV.fetch('GITHUB_TOKEN', 'none')} arg=#{ARGV[0]}\"\n",
        ),
        (
            "/usr/lib/wolos/pwsh",
            "far.ps1",
            "param($Arg)\n\
             \"helper=$env:WOL_HELPER token=$(if ($env:GITHUB_TOKEN) { 'set' } else { 'none' }) arg=$Arg\"\n",
        ),
    ];
    for (prog, name, text) in cases {
        let home = temp_home(&format!("far-{name}"));
        if !ready(prog, &home) {
            eprintln!("{prog} not installed; {name} skipped");
            continue;
        }
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let ssh = bin.join("ssh");
        std::fs::write(
            &ssh,
            "#!/bin/sh\nfor a in \"$@\"; do last=$a; done\nexec /bin/sh -c \"$last\"\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = home.join(name);
        std::fs::write(&file, text).unwrap();
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let state = home.join("state");
        let o = wol(
            &["script", "on", "box", file.to_str().unwrap(), "it's $(x)"],
            &[
                ("HOME", &home),
                ("XDG_STATE_HOME", &state),
                ("PATH", Path::new(&path)),
            ],
        );
        let out = String::from_utf8_lossy(&o.stdout);
        assert!(
            o.status.success(),
            "{name}: {out} {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert_eq!(out, "helper=far token=none arg=it's $(x)\n", "{name}");
        let log = std::fs::read_to_string(state.join("wolos/actions.jsonl")).unwrap();
        assert!(
            log.contains("\"tool\":\"script on\"") && log.contains("\"via\":\"ssh\""),
            "{log}"
        );
    }
}
