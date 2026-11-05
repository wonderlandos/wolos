// SPDX-License-Identifier: GPL-2.0-or-later
//! The OS firewall and the Tor scopes share the kernel's rule set (ADR 0056 D9): neither may delete
//! the other's tables.
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn code_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn the_os_firewall_never_flushes_the_whole_ruleset() {
    let conf = fs::read_to_string(root().join("desktop/overlay/etc/wolos/nftables.conf")).unwrap();
    let lines = code_lines(&conf);
    assert!(
        !lines.iter().any(|l| l.contains("flush ruleset")),
        "`flush ruleset` would delete inet wol_tor and wol_tor_apps on every boot and reload"
    );
    assert!(
        lines.iter().any(|l| l == "destroy table inet wolos"),
        "it replaces only its own table"
    );
    let own_table = lines
        .iter()
        .position(|l| l.starts_with("table inet wolos"))
        .unwrap();
    let destroy = lines
        .iter()
        .position(|l| l == "destroy table inet wolos")
        .unwrap();
    assert!(
        destroy < own_table,
        "destroyed, then defined, in one file (one transaction)"
    );
    assert!(
        !lines.iter().any(|l| l.contains("wol_tor")),
        "the firewall does not know the Tor tables"
    );
}

#[test]
fn no_shipped_nft_file_or_command_flushes_everything() {
    let mut files = Vec::new();
    for d in ["desktop", "files"] {
        walk(&root().join(d), &mut files);
    }
    let mut bad = Vec::new();
    for f in files {
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        let name = f.to_string_lossy();
        if (name.contains("nft") || name.ends_with(".nft") || name.contains("nftables"))
            && code_lines(&text)
                .iter()
                .any(|l| l.contains("flush ruleset"))
        {
            bad.push(name.to_string());
        }
    }
    assert!(bad.is_empty(), "these flush the whole ruleset: {bad:?}");
}

#[test]
fn the_tor_tables_are_ours_and_replace_only_themselves() {
    use wol_tor::transparent::{
        APP_TABLE, SYSTEM_TABLE, SystemGateway, app_host_ruleset, system_ruleset,
    };
    let sys = system_ruleset(&SystemGateway {
        tor_uid: 105,
        lan: vec![],
    })
    .render();
    let apps = app_host_ruleset().render();
    for (text, table) in [(sys, SYSTEM_TABLE), (apps, APP_TABLE)] {
        assert!(text.contains(&format!("delete table inet {table}\n")));
        assert!(!text.contains("flush ruleset") && !text.contains("table inet wolos"));
        assert!(!text.contains("delete table inet wolos"));
    }
}

#[test]
fn the_tor_rules_come_after_the_firewall_and_survive_its_reload() {
    use wol_tor::units::{NFTABLES_DROPIN_NAME, nftables_dropin, rules_unit_file};
    let unit = rules_unit_file(Path::new("/usr/lib/wol-tor"));
    assert!(unit.contains("After=nftables.service"));
    let os_dropin = fs::read_to_string(
        root().join("files/usr/lib/systemd/system/nftables.service.d/wolos.conf"),
    )
    .unwrap();
    assert!(
        os_dropin.contains("ExecReload=\n"),
        "the OS drop-in resets ExecReload"
    );
    assert!(
        NFTABLES_DROPIN_NAME > "wolos.conf",
        "ours sorts after, so the reset does not remove it"
    );
    let d = nftables_dropin(Path::new("/usr/lib/wol-tor"));
    assert!(d.contains("apply --reload-rules"));
}
