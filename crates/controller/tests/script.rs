// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol script` (ADR 0041, docs/modules/scripting.md): a Ruby script is checked and run exactly
//! like a shell script, and so is an rsh script (ADR 0041; rsh has its own repository, github.com/wonderlandos/rsh). Every parity test walks the same
//! cases for `.sh`, `.rb` and `.rsh`: the argv
//! (interpreter + file, never a shell string), the clean environment, the helper name, the audit
//! record, the ssh path (file on stdin) and the refusals. The last tests run the real `wol` binary
//! on real scripts and skip with a message when the interpreter is missing.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
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
        "rsh",
        "tidy.rsh",
        "#!/usr/bin/rsh\necho hi\n",
        "/usr/bin/rsh",
    ),
    (
        "rsh",
        "tidy",
        "#!/usr/bin/env rsh\necho hi\n",
        "/usr/bin/rsh",
    ),
    // a .rb file goes through rsh when its shebang says so (plain Ruby there, unless it opts in)
    (
        "rsh",
        "tidy.rb",
        "#!/usr/bin/rsh\nputs 'hi'\n",
        "/usr/bin/rsh",
    ),
    (
        "rsh",
        "tidy.rb",
        "#!/usr/bin/rsh\n# rsh: shell\necho hi\n",
        "/usr/bin/rsh",
    ),
];

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
        assert_eq!(&argv[at + 1..], [path.as_str(), "a", "--b"], "{kind}");
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
        // rsh reads a .rb file as plain Ruby: on stdin it needs the name to know it is one
        assert_eq!(
            cmd.contains("'--name' 'tidy.rb' '--'"),
            *kind == "rsh" && file.ends_with(".rb"),
            "{kind} {file}: {cmd}"
        );
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
        let syntax = if *prog == "/usr/bin/ruby" {
            "-wc"
        } else {
            "-n"
        };
        assert_eq!(e.execs[0], [prog.to_string(), syntax.into(), path.clone()]);
        assert!(e.out.contains("parses"), "{}", e.out);
        assert!(audit_log(&e).is_empty(), "a check is a read");

        // the linter is installed: it runs after a clean parse
        let (mut e, path) = env_with(&format!("lint{i}"), file, text);
        e.captured = Some("1.0\n".into());
        e.exec_codes = vec![0, 1];
        assert_eq!(run(&format!("script check {path} --json"), &mut e), 65);
        let lint = &e.execs[1];
        let tool = if *prog == "/usr/bin/ruby" {
            assert!(lint.contains(&"--only".to_string()), "{lint:?}");
            assert!(lint.contains(&"--force-default-config".to_string()));
            "rubocop"
        } else if *prog == "/usr/bin/rsh" {
            assert_eq!(lint[1], "--lint", "{lint:?}");
            assert!(!lint.contains(&"--only".to_string()), "{lint:?}");
            "rsh"
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

        // a parse error stops before the linter
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
fn kinds_list_names_ruby_and_rsh_beside_sh() {
    let mut e = FakeEnv::default();
    assert_eq!(run("script --json", &mut e), 0);
    let v: serde_json::Value = serde_json::from_str(&e.out).unwrap();
    let ids: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["sh", "bash", "ruby", "rsh"]);
    assert_eq!(v[2]["extensions"], serde_json::json!([".rb"]));
    assert_eq!(v[3]["extensions"], serde_json::json!([".rsh"]));
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
            "/usr/bin/rsh",
            "probe.rsh",
            "#!/usr/bin/rsh\n\
             puts \"helper=#{ENV['WOL_HELPER']} token=#{ENV.fetch('GITHUB_TOKEN', 'none')}\"\n\
             system(ARGV[0], 'focus', 'on')\nputs \"code=#{$?.exitstatus}\"\n"
                .to_string(),
        ),
    ];
    for (prog, name, text) in cases {
        if !have(prog) {
            eprintln!("{prog} not installed; {name} skipped");
            continue;
        }
        let home = temp_home(&format!("real-{name}"));
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
            "/usr/bin/rsh",
            "#!/usr/bin/rsh\necho ok\n",
            "#!/usr/bin/rsh\nif test -e x\n",
        ),
    ] {
        if !have(prog) {
            eprintln!("{prog} not installed; check skipped");
            continue;
        }
        let home = temp_home(&format!("realchk-{}", prog.replace('/', "")));
        for (n, t, ok) in [("good", good, true), ("bad", bad, false)] {
            let f = home.join(n);
            std::fs::write(&f, t).unwrap();
            let o = wol(&["script", "check", f.to_str().unwrap(), "--json"], &[]);
            let v: serde_json::Value = serde_json::from_slice(&o.stdout)
                .unwrap_or_else(|e| panic!("{prog} {n}: {e}: {:?}", o));
            assert_eq!(v["ok"], ok, "{prog} {n}: {v}");
            assert_eq!(o.status.success(), ok);
        }
    }
}

/// A `.rb` file whose shebang names rsh is plain Ruby (the shell layer is off) unless its first
/// line after the shebang is `# rsh: shell`; `wol script check` and `run` follow the same rule.
#[test]
fn real_rb_files_through_rsh_are_plain_ruby_unless_they_opt_in() {
    if !have("/usr/bin/rsh") {
        eprintln!("/usr/bin/rsh not installed; the .rb mode test is skipped");
        return;
    }
    let home = temp_home("real-rb-mode");
    let state = home.join("state");
    let wol_run = |name: &str, text: &str| {
        let f = home.join(name);
        std::fs::write(&f, text).unwrap();
        wol(
            &["script", "run", f.to_str().unwrap()],
            &[("HOME", &home), ("XDG_STATE_HOME", &state)],
        )
    };
    // plain: `echo hi` is a Ruby call of a method that does not exist
    let plain = "#!/usr/bin/rsh\nbegin\n  echo 'hi'\nrescue NoMethodError\n  puts 'ruby'\nend\n";
    let o = wol_run("plain.rb", plain);
    assert!(o.status.success(), "{o:?}");
    assert_eq!(String::from_utf8_lossy(&o.stdout), "ruby\n");
    // opted in: the same word starts a command line
    let o = wol_run("shell.rb", "#!/usr/bin/rsh\n# rsh: shell\necho hi\n");
    assert!(o.status.success(), "{o:?}");
    assert_eq!(String::from_utf8_lossy(&o.stdout), "hi\n");
    // a command line is a mistake in a plain .rb file, and check says so
    let f = home.join("cmd.rb");
    std::fs::write(&f, "#!/usr/bin/rsh\ncd ~/src/app\n").unwrap();
    let o = wol(&["script", "check", f.to_str().unwrap(), "--json"], &[]);
    assert!(!o.status.success(), "{o:?}");
    std::fs::write(&f, "#!/usr/bin/rsh\n# rsh: shell\ncd ~/src/app\n").unwrap();
    let o = wol(&["script", "check", f.to_str().unwrap(), "--json"], &[]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
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
            "/usr/bin/rsh",
            "far.rsh",
            "#!/usr/bin/rsh\n\
             puts \"helper=#{ENV['WOL_HELPER']} token=#{ENV.fetch('GITHUB_TOKEN', 'none')} arg=#{ARGV[0]}\"\n",
        ),
        // a .rb file with an rsh shebang is plain Ruby over ssh too: `echo` is a Ruby call there
        (
            "/usr/bin/rsh",
            "far.rb",
            "#!/usr/bin/rsh\nbegin\n  echo 'x'\nrescue NoMethodError\n  \
             puts \"helper=#{ENV['WOL_HELPER']} token=#{ENV.fetch('GITHUB_TOKEN', 'none')} arg=#{ARGV[0]}\"\nend\n",
        ),
    ];
    for (prog, name, text) in cases {
        if !have(prog) {
            eprintln!("{prog} not installed; {name} skipped");
            continue;
        }
        let home = temp_home(&format!("far-{name}"));
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
