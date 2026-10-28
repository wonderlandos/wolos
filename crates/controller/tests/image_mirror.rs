//! WolOS mirrors (ADR 0019): repo order in pacman.conf, the wolos-mirrorlist package, the
//! [wolos-cachy] CPU gate (`wol mirror`), mirror-cachy.sh against local fixture databases, the
//! release workflows and manifest.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use std::{fs, path::PathBuf, process::Command};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}
fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}
fn snapshot() -> String {
    read("image/arch/snapshot").trim().to_owned()
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
        "image/arch/pacman.conf",
        "image/arch/airootfs/etc/pacman.conf",
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
    let list = read("image/repo/wolos-mirrorlist/wolos-mirrorlist");
    let s = wol_core::mirror::servers(&list);
    assert_eq!(
        s[0],
        format!("https://wolos.squareweb.app/$repo-{}", snapshot()),
        "primary = the WolOS CDN"
    );
    assert_eq!(
        s[1],
        format!(
            "https://github.com/wonderlandos/wolos/releases/download/$repo-{}",
            snapshot()
        ),
        "fallback = the release assets the CDN pulls from"
    );
    assert!(s.len() >= 2, "extra mirrors listed");
    assert!(
        s.iter().all(|u| u.contains(&snapshot())),
        "every mirror at the release"
    );
    let pb = read("image/repo/wolos-mirrorlist/PKGBUILD");
    assert!(pb.contains(&format!("pkgver={}\n", snapshot().replace('-', "."))));
    assert!(pb.contains("backup=(etc/pacman.d/wolos-mirrorlist)"));
    assert!(read("image/repo/packages").contains("\nwolos-mirrorlist "));
    assert!(read("image/arch/packages.x86_64").contains("\nwolos-mirrorlist\n"));
    // The image build reads [wolos] from the local repo, whatever line names its servers.
    assert!(read("image/build.sh").contains("(Server|Include)"));
}

#[test]
fn gate_turns_cachy_on_only_on_v3_cpus() {
    let conf = read("image/arch/pacman.conf");
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
    let inst = read("image/files/usr/lib/wolos/install-run");
    assert!(inst.contains("wol mirror gate --yes"));
    assert!(inst.contains("wol mirror kernel"));
    assert!(
        inst.find("wol mirror gate").unwrap() < inst.find("say boot").unwrap(),
        "kernel picked before the initramfs and boot entry"
    );
    assert!(read("image/files/usr/lib/wolos/firstboot").contains("wol mirror gate --yes"));
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

/// Same upstream version only, closure through dependencies, sha256 checked, resumable.
#[test]
fn mirror_script_selects_matching_versions_and_reports_sizes() {
    if Command::new("bsdtar").arg("--version").output().is_err() {
        eprintln!("no bsdtar: skipped");
        return;
    }
    let d = scratch("script");
    // repo-add stand-in: records the files it got (the fixtures are not real packages).
    let ra = d.join("repo-add");
    fs::write(
        &ra,
        "#!/bin/sh\nshift\ndb=$1; shift\nprintf '%s\\n' \"$@\" > \"$db\"\n",
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&ra).status().unwrap();
    let (a, c, out) = (d.join("arch"), d.join("cachy"), d.join("out"));
    for x in [&a, &c] {
        fs::create_dir_all(x.join("core")).unwrap();
    }
    for r in ["core", "extra", "multilib"] {
        fs::create_dir_all(a.join(r)).unwrap();
    }
    db(
        &a.join("core"),
        "core",
        &[
            (
                "zstd",
                "1.5.7-3",
                "glibc>=2.40 lz4",
                "zstd-1.5.7-3-x86_64.pkg.tar.zst",
            ),
            ("glibc", "2.44-1", "", "glibc-2.44-1-x86_64.pkg.tar.zst"),
            ("lz4", "1:1.10.0-2", "", "lz4-1:1.10.0-2-x86_64.pkg.tar.zst"),
            ("bash", "5.3-1", "", "bash-5.3-1-x86_64.pkg.tar.zst"),
        ],
    );
    db(&a.join("extra"), "extra", &[]);
    db(&a.join("multilib"), "multilib", &[]);
    for r in ["cachyos-v3", "cachyos-core-v3", "cachyos-extra-v3"] {
        fs::create_dir_all(c.join(r)).unwrap();
    }
    db(
        &c.join("cachyos-v3"),
        "cachyos-v3",
        &[
            (
                "zstd",
                "1.5.7-4",
                "glibc lz4",
                "zstd-1.5.7-4-x86_64_v3.pkg.tar.zst",
            ),
            ("glibc", "2.45-1", "", "glibc-2.45-1-x86_64_v3.pkg.tar.zst"),
            (
                "lz4",
                "1:1.10.0-3",
                "",
                "lz4-1:1.10.0-3-x86_64_v3.pkg.tar.zst",
            ),
            ("bash", "5.3-1.1", "", "bash-5.3-1.1-x86_64_v3.pkg.tar.zst"),
        ],
    );
    db(&c.join("cachyos-core-v3"), "cachyos-core-v3", &[]);
    db(&c.join("cachyos-extra-v3"), "cachyos-extra-v3", &[]);
    for r in ["cachyos-v3"] {
        for f in fs::read_dir(c.join(r)).unwrap().flatten() {
            let n = f.file_name().to_string_lossy().into_owned();
            if n.ends_with(".zst") {
                fs::write(f.path().with_file_name(format!("{n}.sig")), "sig").unwrap();
            }
        }
    }
    let script = |extra: &[(&str, &str)]| {
        let mut cmd = Command::new("bash");
        cmd.arg(repo("image/repo/mirror-cachy.sh"))
            .env("WOLOS_CACHY_DIR", &out)
            .env("WOLOS_CACHY_PKGS", "zstd")
            .env("WOLOS_ARCH_URL", format!("file://{}/$repo", a.display()))
            .env("WOLOS_CACHY_URL", format!("file://{}/$repo", c.display()))
            .env("WOLOS_UNSIGNED", "1")
            .env("WOLOS_CACHY_NOVERIFY", "1")
            .env("WOLOS_REPO_ADD", &ra);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    };
    let o = script(&[]);
    let report = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        o.status.success(),
        "{report}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let sel = fs::read_to_string(out.join("selected.tsv")).unwrap();
    let names: Vec<&str> = sel.lines().map(|l| l.split('\t').next().unwrap()).collect();
    // glibc: 2.45 upstream vs 2.44 in the snapshot -> Arch's stays. bash: not in the closure.
    assert_eq!(names, ["lz4", "zstd"], "{sel}");
    let x = out.join("x86_64");
    assert!(x.join("zstd-1.5.7-4-x86_64_v3.pkg.tar.zst.sig").exists());
    assert!(
        x.join("lz4-1.1.10.0-3-x86_64_v3.pkg.tar.zst").exists(),
        "epoch ':' -> '.'"
    );
    assert_eq!(
        fs::read_to_string(x.join("wolos-cachy.db.tar.zst")).unwrap(),
        "lz4-1.1.10.0-3-x86_64_v3.pkg.tar.zst\nzstd-1.5.7-4-x86_64_v3.pkg.tar.zst\n"
    );
    assert!(report.contains("mirrored 2 packages"), "{report}");
    assert!(report.contains("over 2 GiB: none"), "{report}");
    let rev = fs::read_to_string(out.join("revision")).unwrap();
    assert!(rev.starts_with(&snapshot().replace('-', "")), "{rev}");
    // Resumable and deterministic: a second run downloads nothing and gives the same revision.
    fs::remove_dir_all(c.join("cachyos-v3")).unwrap();
    fs::create_dir_all(c.join("cachyos-v3")).unwrap();
    let again = script(&[]);
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert_eq!(fs::read_to_string(out.join("revision")).unwrap(), rev);
    // A file that does not match the database never enters the repo.
    fs::write(x.join("zstd-1.5.7-4-x86_64_v3.pkg.tar.zst"), "tampered").unwrap();
    let bad = script(&[]);
    assert!(!bad.status.success());
    assert!(!x.join("zstd-1.5.7-4-x86_64_v3.pkg.tar.zst").exists());
    // Plan mode: selection and an estimate, no signing key needed.
    let plan = script(&[("WOLOS_CACHY_PLAN", "1"), ("WOLOS_UNSIGNED", "0")]);
    assert!(String::from_utf8_lossy(&plan.stdout).contains("estimated 2 packages"));
    assert!(
        read("image/repo/mirror-cachy.sh").contains("VALIDSIG"),
        "upstream signatures checked"
    );
    assert!(read("image/repo/cachy-trusted").len() >= 40);
}

/// A tag refreshes [wolos-cachy] and publishes it beside [wolos]; the manifest names it.
#[test]
fn release_refreshes_the_cachy_snapshot() {
    let iso = read(".github/workflows/iso.yml");
    for want in [
        "mirror-cachy.sh",
        "wolos-cachy-",
        "sizes.txt",
        "WOLOS_CACHY_DIR",
    ] {
        assert!(iso.contains(want), "iso.yml lacks {want}");
    }
    let pk = read(".github/workflows/packages.yml");
    assert!(
        pk.contains("tag=wolos-$snap"),
        "[wolos] tag = $repo-<date>, as the mirrorlist reads"
    );
    let d = scratch("manifest");
    let f = d.join("wolos-0.11.2-x86_64.iso");
    fs::write(&f, "iso").unwrap();
    let o = Command::new("bash")
        .arg(repo("image/release-manifest.sh"))
        .args([
            f.to_str().unwrap(),
            "2026-10-20",
            "r42",
            "https://e.org",
            "0.11.2",
            "7.3.0-1",
            "20261020-ab12cd34",
        ])
        .output()
        .unwrap();
    let m = wol_core::update::check::Manifest::parse(&String::from_utf8_lossy(&o.stdout)).unwrap();
    assert_eq!(m.cachy, "20261020-ab12cd34");
    assert!(read("image/build.sh").contains("cachy}/revision"));
}

const WOLOS_KEY: &str = "C3B6062A2BA6EEABC1DA0F5766C5F2B181374033";

fn fingerprints(p: &str) -> Vec<String> {
    let out = Command::new("gpg")
        .args(["--show-keys", "--with-colons"])
        .arg(repo(p))
        .output()
        .expect("gpg");
    let mut fprs = Vec::new();
    let mut primary = false;
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        let f: Vec<&str> = l.split(':').collect();
        match f[0] {
            "pub" => primary = true,
            "sub" => primary = false,
            "fpr" if primary => fprs.push(f[9].to_owned()),
            _ => {}
        }
    }
    fprs
}

#[test]
fn keyring_package_carries_repo_and_cachy_keys_fully_trusted() {
    let cachy = read("image/repo/cachy-trusted");
    let cachy: Vec<&str> = cachy.lines().filter_map(|l| l.split(':').next()).collect();
    let keys = fingerprints("image/repo/wolos-keyring/wolos.gpg");
    assert!(keys.iter().any(|k| k == WOLOS_KEY), "{keys:?}");
    assert_eq!(fingerprints("image/repo/wolos-repo.pub.asc"), [WOLOS_KEY]);
    for c in &cachy {
        assert!(keys.iter().any(|k| k == c), "{c} missing from {keys:?}");
    }
    let trusted = read("image/repo/wolos-keyring/wolos-trusted");
    for k in keys.iter() {
        assert!(
            trusted.contains(&format!("{k}:4:\n")),
            "{k} not fully trusted"
        );
    }
    let pb = read("image/repo/wolos-keyring/PKGBUILD");
    assert!(pb.contains("$pkgdir/usr/share/pacman/keyrings/wolos.gpg"));
    assert!(pb.contains("$pkgdir/usr/share/pacman/keyrings/wolos-trusted"));
    assert!(pb.contains("install=wolos-keyring.install"));
    let inst = read("image/repo/wolos-keyring/wolos-keyring.install");
    assert!(inst.contains("post_install()") && inst.contains("post_upgrade()"));
    assert!(inst.contains("pacman-key --populate wolos"));
}

#[test]
fn keyring_is_built_before_base_and_installed_first() {
    let order = |text: &str, name: &str| {
        text.lines()
            .position(|l| l.split_whitespace().next() == Some(name))
            .unwrap_or_else(|| panic!("{name} missing"))
    };
    let pk = read("image/repo/packages");
    assert!(order(&pk, "wolos-keyring") < order(&pk, "wolos-base"));
    assert!(pk.contains("\nwolos-keyring "));
    let list = read("image/arch/packages.x86_64");
    assert!(order(&list, "wolos-keyring") < order(&list, "wolos-base"));
}
