// SPDX-License-Identifier: GPL-2.0-or-later
//! QA-PLUG-6: `wol` run from Claude Code's Bash tool meets the helper gate (`CLAUDECODE=1` or
//! `WOL_HELPER`, controller::run `Env::helper_name`), and the docs say so, with its limit.
#[path = "support/repo.rs"]
mod repo;

use controller::cli::parse;
use controller::run::{AGENT_ENV, AGENT_HELPER, Env, FakeEnv, execute};

#[test]
fn the_adr_and_the_manual_say_where_the_gate_is() {
    let adr = repo::read("docs/adr/0032-optional-modules.md");
    assert!(
        !adr.contains("Agents change WolOS only through"),
        "ADR 0032 promises the gate to every agent"
    );
    assert!(adr.contains("Claude Code's Bash tool") && adr.contains("WOL_HELPER"));
    assert!(adr.contains("CLAUDECODE=1") && adr.contains("advisory"));
    let manual = repo::read("docs/modules/automation.md");
    let row = manual
        .lines()
        .find(|l| l.contains("Bash tool"))
        .expect("automation.md names an agent's own shell");
    assert!(
        row.contains("CLAUDECODE=1") && row.contains("WOL_HELPER"),
        "{row}"
    );
}

#[test]
fn a_wol_command_without_a_helper_name_runs_as_cli() {
    // What the docs say: no WOL_HELPER, no gate, journal by `cli`; with it, the helper gate.
    let mut e = FakeEnv::default();
    assert_eq!(e.actor(), "cli");
    e.vars.push(("WOL_HELPER".into(), "claude-code".into()));
    assert_eq!(e.actor(), "script:claude-code");
}

/// QA-PLUG-26 / 58 / 59 / 60 / 62: the server's limits are in the manual, with the numbers the code uses.
#[test]
fn the_mcp_limits_are_in_the_manual() {
    let md = repo::read("docs/modules/automation.md");
    let sec = md
        .split("## `wol mcp` server limits")
        .nth(1)
        .and_then(|s| s.split("\n## ").next())
        .expect("a section on the server's limits");
    assert!(
        sec.contains(&format!("{} MiB", controller::mcp::MAX_LINE >> 20)),
        "{sec}"
    );
    assert!(
        sec.contains(&format!("{} KiB", controller::mcp::MAX_OUTPUT >> 10)),
        "{sec}"
    );
    for w in ["-32002", "batch", "root", "`*`", "initialize", "has gone"] {
        assert!(sec.contains(w), "{w}");
    }
}

/// QA-PLUG-6: a `wol` started by an assistant's shell (`CLAUDECODE=1`) is a helper: its change goes
/// to the approval gate (here: no WolOS session, so it is refused and nothing changes) and the
/// audit log names it; the same command from a person's shell runs as `cli`.
#[test]
fn a_wol_command_in_an_assistants_shell_meets_the_helper_gate() {
    let tmp = |tag: &str| {
        let d = std::env::temp_dir().join(format!("wol-agent-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    };
    let run = |e: &mut FakeEnv| execute(&parse(&["appearance", "set", "dark"]).unwrap(), e);
    // A person's shell: no gate.
    let mut person = FakeEnv {
        home: tmp("person"),
        ..Default::default()
    };
    assert_eq!(person.actor(), "cli");
    assert_eq!(run(&mut person), 0, "{}", person.err);
    // The assistant's shell: the gate asks, finds no session, refuses; nothing was changed.
    let mut agent = FakeEnv {
        home: tmp("agent"),
        ..Default::default()
    };
    agent.vars.push((AGENT_ENV.into(), "1".into()));
    assert_eq!(agent.actor(), format!("script:{AGENT_HELPER}"));
    let code = run(&mut agent);
    assert_ne!(code, 0, "{}", agent.out);
    assert!(agent.err.contains("No WolOS session"), "{}", agent.err);
    assert!(!agent.home.join("wolos/theme").exists());
    // WOL_HELPER names it instead when both are set; CLAUDECODE=0 is not an assistant.
    agent.vars.push(("WOL_HELPER".into(), "mine".into()));
    assert_eq!(agent.actor(), "script:mine");
    let mut off = FakeEnv {
        home: tmp("off"),
        ..Default::default()
    };
    off.vars.push((AGENT_ENV.into(), "0".into()));
    assert_eq!(off.actor(), "cli");
}
