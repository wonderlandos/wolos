// SPDX-License-Identifier: GPL-2.0-or-later
//! WolOS mirrors (ADR 0030): repo order in pacman.conf, the wolos-mirrorlist package (the `[wolos]`
//! list and the CachyOS `/cachy/` redirect list), the CPU gate of the three CachyOS x86-64-v3
//! sections (`wol mirror`), status and latency ranking of `[wolos]` only.
#![allow(dead_code, unused_imports)]
#[path = "support/siblings.rs"]
mod siblings;

use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use std::{fs, path::PathBuf};
use wol_core::mirror as m;
fn repo(p: &str) -> PathBuf {
    siblings::root().join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn snapshot() -> String {
    read("profile/snapshot").trim().to_owned()
}

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    env.out.clear();
    env.err.clear();
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

const V3: &str = "flags\t\t: cx16 lahf_lm popcnt pni sse4_1 sse4_2 ssse3 avx avx2 bmi1 bmi2 f16c fma abm movbe xsave\n";

const V2: &str = "flags\t\t: cx16 lahf_lm popcnt pni sse4_1 sse4_2 ssse3\n";

/// `https://<host>` of a `Server = https://<host>/...` value.
fn host(server: &str) -> &str {
    let rest = server
        .strip_prefix("https://")
        .unwrap_or_else(|| panic!("https only: {server}"));
    &server[..8 + rest.find('/').unwrap_or(rest.len())]
}

/// First hit wins: [wolos], then the three CachyOS v3 sections (off until the gate), then Arch at
/// the snapshot. Both pacman.conf copies accept x86_64_v3 packages.
#[test]
fn repos_in_order_with_the_wolos_mirrorlists() {
    for f in ["profile/pacman.conf", "profile/airootfs/etc/pacman.conf"] {
        let c = read(f);
        let on = m::gate(&c, true);
        let repos: Vec<&str> = on
            .lines()
            .filter(|l| l.starts_with('[') && *l != "[options]")
            .collect();
        assert_eq!(
            repos,
            [
                "[wolos]",
                "[cachyos-v3]",
                "[cachyos-core-v3]",
                "[cachyos-extra-v3]",
                "[core]",
                "[extra]",
                "[multilib]"
            ],
            "{f}"
        );
        assert!(!m::cachy_enabled(&c), "{f}: off by default");
        assert_eq!(
            on.matches(&format!("\nInclude = {}\n", m::MIRRORLIST))
                .count(),
            1,
            "{f}"
        );
        assert_eq!(
            on.matches(&format!("\nInclude = {}\n", m::CACHY_MIRRORLIST))
                .count(),
            3,
            "{f}: one include per CachyOS section"
        );
        assert!(
            c.contains("\nArchitecture = x86_64 x86_64_v3\n"),
            "{f}: CachyOS v3 packages are arch x86_64_v3"
        );
    }
    // [wolos]: the CDN first (<domain>/<repo>-<date>/<file>, the bucket layout), the release
    // assets second; every line at the release date.
    let list = read("wolos-mirrorlist/wolos-mirrorlist");
    let s = m::servers(&list);
    assert_eq!(s.len(), 2, "{s:?}");
    assert_eq!(
        s[0],
        format!("{}/$repo-{}", host(&s[0]), snapshot()),
        "primary = the WolOS CDN"
    );
    let cdn = host(&s[0]).to_owned();
    assert!(
        cdn == "https://REPO_DOMAIN" || cdn.ends_with(".cloudfront.net"),
        "{cdn}: the CloudFront domain (placeholder until terraform apply)"
    );
    assert_eq!(
        s[1],
        format!(
            "https://github.com/wonderlandos/wolos-packages/releases/download/$repo-{}",
            snapshot()
        ),
        "fallback = the release assets the CDN falls back to as well"
    );
    assert!(
        !list.contains("squareweb"),
        "the old CDN is gone (ADR 0030)"
    );
    // The CachyOS list: one server, the same CDN's /cachy/ redirect; no date (the repos roll).
    let cachy = read("wolos-mirrorlist/wolos-cachy-mirrorlist");
    assert_eq!(m::servers(&cachy), [format!("{cdn}/cachy/$repo")]);
    assert!(
        !cachy.contains(&snapshot()),
        "no release date in the CachyOS list"
    );
    let pb = read("wolos-mirrorlist/PKGBUILD");
    assert!(pb.contains(&format!("pkgver={}\n", snapshot().replace('-', "."))));
    assert!(
        pb.contains("backup=(etc/pacman.d/wolos-mirrorlist etc/pacman.d/wolos-cachy-mirrorlist)")
    );
    assert!(pb.contains("source=(wolos-mirrorlist wolos-cachy-mirrorlist)"));
    assert!(pb.contains(&format!("\"$pkgdir{}\"", m::MIRRORLIST)));
    assert!(pb.contains(&format!("\"$pkgdir{}\"", m::CACHY_MIRRORLIST)));
    assert!(read("packages").contains("\nwolos-mirrorlist "));
    assert!(read("profile/packages.x86_64").contains("\nwolos-mirrorlist\n"));
    // The image build reads [wolos] from the local repo, whatever line names its servers.
    assert!(read("build.sh").contains("(Server|Include)"));
}

#[test]
fn gate_turns_the_cachyos_sections_on_only_on_v3_cpus() {
    let conf = read("profile/pacman.conf");
    let mut e = FakeEnv {
        root: true,
        files: vec![
            ("/proc/cpuinfo".into(), V3.into()),
            ("/etc/pacman.conf".into(), conf.clone()),
        ],
        ..Default::default()
    };
    assert_ne!(run("mirror gate", &mut e), 0, "needs --yes");
    assert_eq!(run("mirror gate --yes", &mut e), 0, "{}", e.err);
    assert_eq!(e.out, "{\"cachyos\":\"on\"}\n");
    let (p, t) = e.written.last().unwrap();
    assert_eq!(p, &PathBuf::from("/etc/pacman.conf"));
    assert!(m::cachy_enabled(t));
    for r in m::CACHY_REPOS {
        assert!(
            t.contains(&format!("\n[{r}]\nInclude = {}\n", m::CACHY_MIRRORLIST)),
            "{r}"
        );
    }
    // Older CPU: the sections stay off, nothing written.
    let mut old = FakeEnv {
        root: true,
        files: vec![
            ("/proc/cpuinfo".into(), V2.into()),
            ("/etc/pacman.conf".into(), conf),
        ],
        ..Default::default()
    };
    assert_eq!(run("mirror gate --yes", &mut old), 0, "{}", old.err);
    assert!(old.written.is_empty());
    assert_eq!(old.out, "{\"cachyos\":\"off\"}\n");
    assert_eq!(run("mirror kernel", &mut old), 0);
    assert_eq!(old.out, "linux-wolos\n");
    e.captures.push((
        "pacman -Si linux-wolos-v3".into(),
        "Name : linux-wolos-v3\n".into(),
    ));
    assert_eq!(run("mirror kernel", &mut e), 0);
    assert_eq!(e.out, "linux-wolos-v3\n");
    // Installer and first boot both run the gate.
    let inst = read("files/usr/lib/wolos/install-run");
    assert!(inst.contains("wol mirror gate --yes"));
    assert!(inst.contains("wol mirror kernel"));
    assert!(
        inst.find("wol mirror gate").unwrap() < inst.find("say boot").unwrap(),
        "kernel picked before the initramfs and boot entry"
    );
    assert!(read("files/usr/lib/wolos/firstboot").contains("wol mirror gate --yes"));
}

/// Only the [wolos] list is probed and reordered; the CachyOS list is never touched.
#[test]
fn status_and_rank_by_latency_touch_only_the_wolos_list() {
    let list = "# m\nServer = https://a.example/$repo-2026-10-20\nServer = https://b.example/2026-10-20/$repo/$arch\n";
    let cachy = "# c\nServer = https://cdn.example/cachy/$repo\n";
    let on = m::gate(&read("profile/pacman.conf"), true);
    let mut e = FakeEnv {
        root: true,
        files: vec![
            ("/proc/cpuinfo".into(), V3.into()),
            ("/etc/pacman.conf".into(), on),
            ("/etc/pacman.d/wolos-mirrorlist".into(), list.into()),
            ("/etc/pacman.d/wolos-cachy-mirrorlist".into(), cachy.into()),
        ],
        captures: vec![
            (
                "probe https://a.example/wolos-2026-10-20/wolos.db".into(),
                "80".into(),
            ),
            (
                "probe https://b.example/2026-10-20/wolos/x86_64/wolos.db".into(),
                "9".into(),
            ),
        ],
        ..Default::default()
    };
    assert_eq!(run("mirror status --json", &mut e), 0, "{}", e.err);
    let v: serde_json::Value = serde_json::from_str(&e.out).unwrap();
    assert_eq!(
        (v["level"].as_u64(), v["cachy_enabled"].as_bool()),
        (Some(3), Some(true))
    );
    assert_eq!(v["servers"].as_array().unwrap().len(), 2);
    assert_eq!(run("mirror rank --json", &mut e), 0, "{}", e.err);
    let r: serde_json::Value = serde_json::from_str(&e.out).unwrap();
    assert_eq!(r[0]["ms"], 9);
    assert!(e.written.is_empty(), "no --yes: print only");
    assert_eq!(run("mirror rank --yes --json", &mut e), 0, "{}", e.err);
    assert_eq!(e.written.len(), 1, "one file written: {:?}", e.written);
    let (p, t) = e.written.last().unwrap();
    assert_eq!(p, &PathBuf::from("/etc/pacman.d/wolos-mirrorlist"));
    assert_eq!(
        t,
        "# m\nServer = https://b.example/2026-10-20/$repo/$arch\nServer = https://a.example/$repo-2026-10-20\n"
    );
    e.root = false;
    assert_ne!(run("mirror rank --yes", &mut e), 0, "writing needs root");
    // The real probe follows redirects: the CDN and the release assets answer with 302s.
    let src = read("crates/controller/src/mirror.rs");
    assert!(src.contains("\"-fsSL\""), "curl -L in curl_probe");
}
