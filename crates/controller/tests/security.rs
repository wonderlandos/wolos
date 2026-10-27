//! `wol firewall` and `wol security` (wol_core::security).
use controller::cli::parse;
use controller::run::{FakeEnv, execute};

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn c(cmd: &str, out: &str) -> (String, String) {
    (cmd.into(), out.into())
}

#[test]
fn firewall_on_and_off_go_through_wol_admind() {
    for (line, wire) in [
        ("firewall on", &b"firewall\x001\x00on\x00"[..]),
        ("firewall off", b"firewall\x001\x00off\x00"),
    ] {
        let mut env = FakeEnv {
            replies: vec!["ok\n".into()],
            ..FakeEnv::default()
        };
        assert_eq!(run(line, &mut env), 0, "{line}");
        assert_eq!(env.requests[0].1, wire);
    }
    assert!(parse(&["firewall", "maybe"]).is_err());
}

#[test]
fn status_shows_firewall_and_hardening() {
    let mut env = FakeEnv {
        captures: vec![
            c("sysrc -n pf_enable", "YES\n"),
            c("sysrc -n syslogd_flags", "-ss\n"),
            c("sysctl -n security.bsd.see_other_uids", "0\n"),
            c("sysctl -n kern.randompid", "1\n"),
        ],
        ..FakeEnv::default()
    };
    assert_eq!(run("firewall --json", &mut env), 0);
    let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
    assert_eq!(v["on"], true);
    assert_eq!(
        v["running"],
        serde_json::Value::Null,
        "pfctl: administrators only"
    );
    env.out.clear();
    env.tty = true;
    assert_eq!(run("security", &mut env), 0);
    assert!(
        env.out
            .starts_with("firewall: on (nothing comes in unasked)\n"),
        "{}",
        env.out
    );
    assert!(env.out.contains("system log on the network: no\n"));
    assert!(
        env.out
            .contains("security.bsd.see_other_uids: 0 (want 0, ok)\n")
    );
    assert!(
        env.out
            .contains("security.bsd.see_other_gids: ? (want 0, differs)\n")
    );
}
