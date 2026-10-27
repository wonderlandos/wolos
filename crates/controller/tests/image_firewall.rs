//! Settings > Network > Firewall loads the shipped pf ruleset (`wol-admind firewall on`): nothing
//! comes in unasked except what a desktop needs to work (ping, DHCP, IPv6 neighbours, mDNS).
fn rules() -> String {
    let rel = wol_core::sharing::PF_RULES.trim_start_matches('/');
    let f = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../desktop/overlay")
        .join(rel);
    std::fs::read_to_string(&f).unwrap_or_else(|_| panic!("{} missing", f.display()))
}

fn rule_lines(s: &str) -> Vec<String> {
    s.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn firewall_blocks_unasked_traffic_in() {
    let s = rules();
    assert!(s.starts_with("# SPDX-License-Identifier: BSD-2-Clause\n"));
    assert!(s.contains("Copyright (c) 2026 Julia Klee"));
    let r = rule_lines(&s);
    assert!(r.contains(&"set skip on lo0".to_string()), "{r:?}");
    assert!(r.contains(&"block in all".to_string()), "{r:?}");
    assert!(r.contains(&"pass out all keep state".to_string()), "{r:?}");
    let block = r.iter().position(|l| l == "block in all").unwrap();
    let allowed = [
        "pass in inet proto icmp icmp-type { echoreq, unreach }",
        "pass in inet6 proto icmp6 icmp6-type { echoreq, unreach, toobig, neighbrsol, neighbradv, routeradv }",
        "pass in inet proto udp from port 67 to port 68",
        "pass in proto udp to port 5353",
    ];
    let pass_in: Vec<&String> = r.iter().filter(|l| l.starts_with("pass in")).collect();
    assert_eq!(pass_in.len(), allowed.len(), "{pass_in:?}");
    for a in allowed {
        let i = r
            .iter()
            .position(|l| l == a)
            .unwrap_or_else(|| panic!("{a}"));
        assert!(i > block, "{a} must come after block in (last match wins)");
    }
    assert!(
        !r.iter()
            .any(|l| l.contains("quick") || l.starts_with("pass in all")),
        "{r:?}"
    );
}
