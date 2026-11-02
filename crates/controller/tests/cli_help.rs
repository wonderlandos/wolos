// SPDX-License-Identifier: GPL-2.0-or-later
//! `--help` on any command prints usage and runs nothing (audit2 CLI-1: `wol power shutdown
//! --help` used to shut the computer down). Walks every path in `cli::TREE`.
use controller::cli::{TREE, parse};
use controller::run::{FakeEnv, execute};

fn assert_inert(argv: &[&str]) {
    let inv = parse(argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
    let mut env = FakeEnv::default();
    // A helper context must not turn --help into a gated run either.
    env.vars.push(("WOL_HELPER".into(), "probe".into()));
    assert_eq!(execute(&inv, &mut env), 0, "{argv:?}: {}", env.err);
    assert!(env.out.starts_with("usage: wol"), "{argv:?}: {}", env.out);
    assert!(env.requests.is_empty(), "{argv:?} sent {:?}", env.requests);
    assert!(env.execs.is_empty(), "{argv:?} ran {:?}", env.execs);
    assert!(env.written.is_empty(), "{argv:?} wrote {:?}", env.written);
    assert!(
        env.admin.lock().unwrap().sent.is_empty(),
        "{argv:?} asked wol-admind"
    );
}

#[test]
fn help_flag_runs_nothing_on_every_command() {
    for c in TREE {
        let mut argv: Vec<&str> = c.path.to_vec();
        argv.push("--help");
        assert_inert(&argv);
        // With a value for every argument and the command's own flags, still inert.
        let mut full: Vec<&str> = c.path.to_vec();
        for a in c.args {
            full.push(a.choices.first().copied().unwrap_or("x"));
        }
        let flags: Vec<String> = c.flags.iter().map(|f| format!("--{f}")).collect();
        full.extend(flags.iter().map(String::as_str));
        full.extend(["--yes", "--help"]);
        assert_inert(&full);
    }
}

#[test]
fn power_shutdown_help_prints_its_usage() {
    let inv = parse(&["power", "shutdown", "--help"]).unwrap();
    let mut env = FakeEnv::default();
    assert_eq!(execute(&inv, &mut env), 0);
    assert!(env.out.contains("power shutdown"), "{}", env.out);
    assert!(!env.out.contains("  theme"), "{}", env.out);
    assert!(env.requests.is_empty() && env.execs.is_empty());
}
