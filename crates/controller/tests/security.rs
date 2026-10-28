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
            c(
                "systemctl show -p UnitFileState --value nftables.service",
                "enabled\n",
            ),
            c(
                "systemctl show -p UnitFileState --value rsyslog.service",
                "\n",
            ),
            c(
                "systemctl show -p UnitFileState --value syslog-ng@default.service",
                "\n",
            ),
            c("cat /proc/sys/kernel/kptr_restrict", "2\n"),
            c("cat /proc/sys/kernel/dmesg_restrict", "1\n"),
        ],
        ..FakeEnv::default()
    };
    assert_eq!(run("firewall --json", &mut env), 0);
    let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
    assert_eq!(v["on"], true);
    assert_eq!(v["running"], serde_json::Value::Null, "state unknown");
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
    assert!(env.out.contains("kernel.kptr_restrict: 2 (want 2, ok)\n"));
    assert!(
        env.out
            .contains("fs.protected_symlinks: ? (want 1, differs)\n")
    );
}
