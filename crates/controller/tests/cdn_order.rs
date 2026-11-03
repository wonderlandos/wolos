// SPDX-License-Identifier: GPL-2.0-or-later
//! The CDN server has two names (wol_core::cdn): the shell scripts that read or post to it try
//! `cdn.wolos.org` first, `wolos.squareweb.app` next and, for the release manifest, GitHub last;
//! their defaults are the constants. Scripts run against a fake curl that knows which names are
//! down, so nothing here touches the network.
#[path = "support/siblings.rs"]
mod siblings;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn repo(p: &str) -> PathBuf {
    siblings::root().join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-cdn-order-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// The default of `VAR=${ENV:-default}` or `VAR=${ENV-default}` in a script.
fn default_of(script: &str, var: &str) -> String {
    let line = script
        .lines()
        .find(|l| l.starts_with(&format!("{var}=${{")))
        .unwrap_or_else(|| panic!("no {var}= line"));
    let inner = line.split_once("${").unwrap().1.strip_suffix('}').unwrap();
    let (_, d) = inner.split_once('-').unwrap();
    d.strip_prefix('-').unwrap_or(d).to_string()
}

/// DIR/bin with a fake curl (and quiet pacman, checkupdates) that logs "curl <url>" to DIR/log.
/// Hosts in `down` fail as an unreachable name does (exit 6), hosts in `missing` as a 404 does
/// with -f (exit 22); any other host answers: the file given with -o gets `{"version":"9"}`, a
/// `.sig` prints SIG, a POST body is appended to DIR/sent. Returns PATH.
fn fakes(d: &Path, down: &[&str], missing: &[&str]) -> String {
    let bin = d.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let cases = |hosts: &[&str], code: u8| {
        hosts
            .iter()
            .map(|h| format!("{h}/*) exit {code} ;;\n"))
            .collect::<String>()
    };
    let curl = format!(
        "#!/bin/sh\nfor a; do u=$a; done\necho \"curl $u\" >> {log}\ncase $u in\n{down}{missing}esac\n\
         out=; prev=; post=; for a; do [ \"$prev\" = -o ] && out=$a; [ \"$a\" = --data-binary ] && post=1; prev=$a; done\n\
         if [ -n \"$post\" ]; then cat >> {sent}; exit 0; fi\n\
         case $u in *.sig) printf SIG ;; *) [ -n \"$out\" ] && echo '{{\"version\":\"9\"}}' > \"$out\" ;; esac\n",
        log = d.join("log").display(),
        sent = d.join("sent").display(),
        down = cases(down, 6),
        missing = cases(missing, 22),
    );
    for (n, body) in [
        ("curl", curl.as_str()),
        ("checkupdates", "#!/bin/sh\nexit 2\n"),
        ("pacman", "#!/bin/sh\nexit 0\n"),
        ("arch-audit", "#!/bin/sh\nexit 0\n"),
        (
            "systemctl",
            "#!/bin/sh\necho 'wold.service loaded failed failed WolOS daemon'\n",
        ),
        ("coredumpctl", "#!/bin/sh\nexit 0\n"),
    ] {
        let p = bin.join(n);
        fs::write(&p, body).unwrap();
        Command::new("chmod").arg("+x").arg(&p).status().unwrap();
    }
    format!("{}:/usr/bin:/bin", bin.display())
}

fn asked(d: &Path) -> Vec<String> {
    fs::read_to_string(d.join("log"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim_start_matches("curl ").to_string())
        .collect()
}

const PRIMARY: &str = "https://cdn.wolos.org";
const MIRROR: &str = "https://wolos.squareweb.app";
const GITHUB: &str = "https://github.com";

#[test]
fn the_scripts_defaults_are_the_cdn_constants() {
    let check = read("files/usr/lib/wolos/update-check");
    assert_eq!(
        ["URL", "MIRROR", "FALLBACK"].map(|v| default_of(&check, v)),
        wol_core::cdn::latest("release.json").as_slice(),
        "update-check URL, MIRROR, FALLBACK"
    );
    let report = read("files/usr/lib/wolos/report");
    assert_eq!(
        ["URL", "MIRROR"].map(|v| default_of(&report, v)),
        wol_core::cdn::REPORT,
        "report URL, MIRROR"
    );
    assert_eq!(wol_core::cdn::HOSTS, [PRIMARY, MIRROR]);
}

fn update_check(d: &Path, path: &str) -> PathBuf {
    let out = d.join("out");
    let st = Command::new("sh")
        .arg(repo("files/usr/lib/wolos/update-check"))
        .arg("now")
        .env("PATH", path)
        .env("WOLOS_UPDATES", &out)
        .status()
        .unwrap();
    assert!(st.success());
    out
}

/// While cdn.wolos.org's DNS is not live and the server's own name has no manifest, the check
/// reads GitHub's, asking in order; once the new name answers, it is the only one asked.
#[test]
fn update_check_asks_the_new_name_then_the_old_then_github() {
    let latest = wol_core::cdn::latest("release.json");
    let d = scratch("check-fallback");
    let out = update_check(&d, &fakes(&d, &[PRIMARY], &[MIRROR]));
    assert_eq!(
        asked(&d),
        [
            latest[0].clone(),
            latest[1].clone(),
            latest[2].clone(),
            format!("{}.sig", latest[2]),
        ]
    );
    assert!(latest[2].starts_with(GITHUB));
    assert_eq!(
        fs::read_to_string(out.join("base")).unwrap(),
        "{\"version\":\"9\"}\n"
    );
    assert_eq!(fs::read_to_string(out.join("base.sig")).unwrap(), "534947");

    let d = scratch("check-mirror");
    update_check(&d, &fakes(&d, &[PRIMARY], &[]));
    assert_eq!(
        asked(&d),
        [
            latest[0].clone(),
            latest[1].clone(),
            format!("{}.sig", latest[1])
        ]
    );

    let d = scratch("check-live");
    let out = update_check(&d, &fakes(&d, &[], &[]));
    assert_eq!(asked(&d), [latest[0].clone(), format!("{}.sig", latest[0])]);
    assert!(out.join("base").exists());

    // Nothing answers: no manifest is kept from a half read, the check still finishes.
    let d = scratch("check-none");
    let out = update_check(&d, &fakes(&d, &[PRIMARY, MIRROR, GITHUB], &[]));
    assert_eq!(asked(&d).len(), 3);
    assert!(!out.join("base").exists() && !out.join("base.new").exists());
    assert!(out.join("checked").exists());
}

fn report(d: &Path, path: &str) {
    let conf = d.join("reports.conf");
    fs::write(&conf, "send=1\n").unwrap();
    let osr = d.join("os-release");
    fs::write(&osr, "NAME=WolOS\nVERSION_ID=0.12.1\n").unwrap();
    let st = Command::new("sh")
        .arg(repo("files/usr/lib/wolos/report"))
        .env("PATH", path)
        .env("WOLOS_REPORTS_CONF", &conf)
        .env("WOLOS_REPORTS_STATE", d.join("state"))
        .env("WOLOS_OS_RELEASE", &osr)
        .status()
        .unwrap();
    assert!(st.success());
}

/// A report goes to cdn.wolos.org, or to the server's own name when the new one cannot be
/// reached; it is sent once either way. When neither takes it, it is tried again next run.
#[test]
fn a_report_goes_to_the_new_name_then_the_old() {
    let [first, second] = wol_core::cdn::REPORT;
    let d = scratch("report-fallback");
    let path = fakes(&d, &[PRIMARY], &[]);
    report(&d, &path);
    assert_eq!(asked(&d), [first, second]);
    let sent = fs::read_to_string(d.join("sent")).unwrap();
    assert_eq!(sent.lines().count(), 1, "{sent}");
    assert!(sent.contains(r#""detail":"wold.service""#), "{sent}");
    report(&d, &path);
    assert_eq!(asked(&d).len(), 2, "sent once");

    let d = scratch("report-live");
    report(&d, &fakes(&d, &[], &[]));
    assert_eq!(asked(&d), [first]);

    let d = scratch("report-none");
    let path = fakes(&d, &[PRIMARY, MIRROR], &[]);
    report(&d, &path);
    report(&d, &path);
    assert_eq!(
        asked(&d),
        [first, second, first, second],
        "kept for the next run"
    );
}
