// SPDX-License-Identifier: GPL-2.0-or-later
//! WolOS mirrors (ADR 0019): repo order in pacman.conf, the wolos-mirrorlist package, the
//! [wolos-cachy] CPU gate (`wol mirror`), mirror-cachy.sh against local fixture databases, the
//! release workflows and manifest.
#![allow(dead_code, unused_imports)]
#[path = "support/siblings.rs"]
mod siblings;

use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use std::{fs, path::PathBuf, process::Command};
fn repo(p: &str) -> PathBuf {
    siblings::root()
        .join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn snapshot() -> String {
    read("profile/snapshot").trim().to_owned()
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-mirror-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    env.out.clear();
    env.err.clear();
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

const V3: &str = "flags\t\t: cx16 lahf_lm popcnt pni sse4_1 sse4_2 ssse3 avx avx2 bmi1 bmi2 f16c fma abm movbe xsave\n";

const V2: &str = "flags\t\t: cx16 lahf_lm popcnt pni sse4_1 sse4_2 ssse3\n";

/// First hit wins: [wolos], then [wolos-cachy] (off until the gate), then Arch at the snapshot.
#[test]
fn repos_in_order_with_the_wolos_mirrorlist() {
    for f in [
        "profile/pacman.conf",
        "profile/airootfs/etc/pacman.conf",
    ] {
        let c = read(f);
        let on = wol_core::mirror::gate(&c, true);
        let repos: Vec<&str> = on
            .lines()
            .filter(|l| l.starts_with('[') && *l != "[options]")
            .collect();
        assert_eq!(
            repos,
            [
                "[wolos]",
                "[wolos-cachy]",
                "[core]",
                "[extra]",
                "[multilib]"
            ],
            "{f}"
        );
        assert!(!wol_core::mirror::cachy_enabled(&c), "{f}: off by default");
        assert_eq!(
            on.matches("Include = /etc/pacman.d/wolos-mirrorlist\n")
                .count(),
            2,
            "{f}"
        );
    }
    let list = read("wolos-mirrorlist/wolos-mirrorlist");
    let s = wol_core::mirror::servers(&list);
    assert_eq!(
        s[0],
        format!("https://wolos.squareweb.app/$repo-{}", snapshot()),
        "primary = the WolOS CDN"
    );
    assert_eq!(
        s[1],
        format!(
            "https://github.com/wonderlandos/wolos-packages/releases/download/$repo-{}",
            snapshot()
        ),
        "fallback = the release assets the CDN pulls from"
    );
    assert!(s.len() >= 2, "extra mirrors listed");
    assert!(
        s.iter().all(|u| u.contains(&snapshot())),
        "every mirror at the release"
    );
    let pb = read("wolos-mirrorlist/PKGBUILD");
    assert!(pb.contains(&format!("pkgver={}\n", snapshot().replace('-', "."))));
    assert!(pb.contains("backup=(etc/pacman.d/wolos-mirrorlist)"));
    assert!(read("packages").contains("\nwolos-mirrorlist "));
    assert!(read("profile/packages.x86_64").contains("\nwolos-mirrorlist\n"));
    // The image build reads [wolos] from the local repo, whatever line names its servers.
    assert!(read("build.sh").contains("(Server|Include)"));
}

#[test]
fn gate_turns_cachy_on_only_on_v3_cpus() {
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
    assert_eq!(e.out, "{\"wolos_cachy\":\"on\"}\n");
    let (p, t) = e.written.last().unwrap();
    assert_eq!(p, &PathBuf::from("/etc/pacman.conf"));
    assert!(wol_core::mirror::cachy_enabled(t));
    // Older CPU: the section stays off, nothing written.
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

#[test]
fn status_and_rank_by_latency() {
    let list = "# m\nServer = https://a.example/$repo-2026-10-20\nServer = https://b.example/2026-10-20/$repo/$arch\n";
    let mut e = FakeEnv {
        root: true,
        files: vec![
            ("/proc/cpuinfo".into(), V3.into()),
            ("/etc/pacman.conf".into(), "[wolos-cachy]\n".into()),
            ("/etc/pacman.d/wolos-mirrorlist".into(), list.into()),
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
    let (_, t) = e.written.last().unwrap();
    assert_eq!(
        t,
        "# m\nServer = https://b.example/2026-10-20/$repo/$arch\nServer = https://a.example/$repo-2026-10-20\n"
    );
    e.root = false;
    assert_ne!(run("mirror rank --yes", &mut e), 0, "writing needs root");
}

/// A sync database: one `<name>-<ver>/desc` per package.
fn db(dir: &std::path::Path, name: &str, pkgs: &[(&str, &str, &str, &str)]) {
    let t = dir.join(format!("t-{name}"));
    fs::create_dir_all(&t).unwrap();
    for (n, v, deps, file) in pkgs {
        let d = t.join(format!("{n}-{v}"));
        fs::create_dir_all(&d).unwrap();
        let bytes = format!("pkg {n} {v}");
        let sum = {
            let o = Command::new("sha256sum")
                .arg("-")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            use std::io::Write;
            o.stdin
                .as_ref()
                .unwrap()
                .write_all(bytes.as_bytes())
                .unwrap();
            String::from_utf8(o.wait_with_output().unwrap().stdout).unwrap()[..64].to_owned()
        };
        fs::write(dir.join(file.replace("%3A", ":")), &bytes).unwrap();
        let mut desc = format!(
            "%FILENAME%\n{file}\n\n%NAME%\n{n}\n\n%VERSION%\n{v}\n\n%CSIZE%\n{}\n\n%SHA256SUM%\n{sum}\n\n",
            bytes.len()
        );
        if !deps.is_empty() {
            desc.push_str(&format!("%DEPENDS%\n{}\n\n", deps.replace(' ', "\n")));
        }
        fs::write(d.join("desc"), desc).unwrap();
    }
    let ok = Command::new("bsdtar")
        .arg("-czf")
        .arg(dir.join(format!("{name}.db")))
        .arg("-C")
        .arg(&t)
        .arg(".")
        .status()
        .unwrap()
        .success();
    assert!(ok);
}
