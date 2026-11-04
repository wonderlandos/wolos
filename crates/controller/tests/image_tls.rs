// SPDX-License-Identifier: GPL-2.0-or-later
//! ADR 0055, the system-wide TLS policy as the image ships it: the OpenSSL and GnuTLS policy files
//! hold `wol_core::net`'s values, `tls-policy` points the packages' own config files at them
//! (idempotent, only its own lines, never over somebody else's setting), the pacman hook runs it
//! after every openssl/gnutls/policy change, and no browser flag file weakens TLS. The container
//! test (ignored: docker or podman and the network) applies all of it to a pristine Arch and checks
//! that OpenSSL, curl and GnuTLS negotiate X25519MLKEM768 with a post-quantum server.
#[path = "support/repo.rs"]
mod repo;
use repo::read;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use wol_core::net;

/// `key = value` of one OpenSSL config section.
fn section(text: &str, name: &str) -> Vec<(String, String)> {
    let head = format!("[{name}]");
    let mut inside = false;
    let mut kv = Vec::new();
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            inside = l == head;
            continue;
        }
        if !inside || l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            kv.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    kv
}

#[test]
fn openssl_policy_file_holds_the_policy() {
    let t = read("files/usr/share/wolos/tls/openssl.cnf");
    assert_eq!(
        section(&t, "wolos_ssl_conf"),
        [("system_default".to_string(), "wolos_tls_policy".to_string())]
    );
    let p = section(&t, "wolos_tls_policy");
    let want = [
        ("MinProtocol", net::OPENSSL_MIN_PROTOCOL),
        ("Groups", net::OPENSSL_GROUPS),
        ("Ciphersuites", net::OPENSSL_TLS13_SUITES),
        ("CipherString", net::OPENSSL_TLS12_CIPHERS),
        ("Options", net::OPENSSL_OPTIONS),
    ];
    let got: Vec<(&str, &str)> = p.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(got, want, "files/usr/share/wolos/tls/openssl.cnf");
}

#[test]
fn gnutls_policy_file_holds_the_policy() {
    let t = read("files/usr/share/wolos/tls/gnutls.priority");
    let lines: Vec<&str> = t
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .collect();
    assert_eq!(lines, [net::GNUTLS_PRIORITY]);
}

/// The hook runs the script after openssl, gnutls or the policy changes, and only then.
#[test]
fn pacman_hook_reapplies_the_policy() {
    let h = read("files/usr/share/libalpm/hooks/wolos-tls.hook");
    for want in [
        "Type = Path",
        "Operation = Install",
        "Operation = Upgrade",
        "Target = etc/ssl/openssl.cnf",
        "Target = etc/gnutls/config",
        "Target = usr/lib/wolos/tls-policy",
        "Target = usr/share/wolos/tls/*",
        "When = PostTransaction",
        "Exec = /usr/lib/wolos/tls-policy",
    ] {
        assert!(h.lines().any(|l| l == want), "{want} missing:\n{h}");
    }
}

/// Arch's /etc/ssl/openssl.cnf (openssl 3.6.5-1) as far as the script cares: a default section
/// naming [openssl_init], the providers, and sections after it.
const ARCH_OPENSSL_CNF: &str = "# OpenSSL example configuration file.\n\
HOME\t\t\t= .\n\
openssl_conf = openssl_init\n\
\n\
[openssl_init]\n\
providers = provider_sect\n\
\n\
[provider_sect]\n\
default = default_sect\n\
\n\
[default_sect]\n\
# activate = 1\n\
\n\
[ req ]\n\
default_bits\t\t= 2048\n";

/// Arch's /etc/gnutls/config (gnutls 3.8.13-3), comments shortened.
const ARCH_GNUTLS_CONFIG: &str =
    "# GnuTLS is built with --enable-ktls\n\n[global]\nktls = false\n#ktls = true\n";

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn root(tag: &str, ossl: &str, gtls: &str) -> Root {
    let d = std::env::temp_dir().join(format!("wolos-tls-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("etc/ssl")).unwrap();
    fs::create_dir_all(d.join("etc/gnutls")).unwrap();
    fs::create_dir_all(d.join("usr/share/wolos/tls")).unwrap();
    for f in ["openssl.cnf", "gnutls.priority"] {
        fs::copy(
            repo::repo(&format!("files/usr/share/wolos/tls/{f}")),
            d.join("usr/share/wolos/tls").join(f),
        )
        .unwrap();
    }
    fs::write(d.join("etc/ssl/openssl.cnf"), ossl).unwrap();
    fs::write(d.join("etc/gnutls/config"), gtls).unwrap();
    Root(d)
}

fn apply(r: &Path) -> (i32, String) {
    run(r, &[])
}

fn run(r: &Path, args: &[&str]) -> (i32, String) {
    let o = Command::new("sh")
        .arg(repo::repo("files/usr/lib/wolos/tls-policy"))
        .args(args)
        .env("WOLOS_TLS_ROOT", r)
        .output()
        .unwrap();
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn text(r: &Path, p: &str) -> String {
    fs::read_to_string(r.join(p)).unwrap()
}

/// The lines the script adds, and only those; a second and third run change nothing.
#[test]
fn tls_policy_points_the_package_configs_at_the_policy_once() {
    let r = root("once", ARCH_OPENSSL_CNF, ARCH_GNUTLS_CONFIG);
    assert_eq!(apply(&r.0), (0, String::new()));
    let o = text(&r.0, "etc/ssl/openssl.cnf");
    let g = text(&r.0, "etc/gnutls/config");
    for _ in 0..2 {
        assert_eq!(apply(&r.0).0, 0);
        assert_eq!(text(&r.0, "etc/ssl/openssl.cnf"), o, "idempotent");
        assert_eq!(text(&r.0, "etc/gnutls/config"), g, "idempotent");
    }
    // OpenSSL: ssl_conf right under [openssl_init], the include at the end, the rest as it was.
    let init = o.find(
        "[openssl_init]\n# wolos-tls\nssl_conf = wolos_ssl_conf\nproviders = provider_sect\n",
    );
    assert!(init.is_some(), "{o}");
    assert!(
        o.ends_with("\n# wolos-tls\n.include /usr/share/wolos/tls/openssl.cnf\n"),
        "{o}"
    );
    let without: String = {
        let mut skip = false;
        o.lines()
            .filter(|l| {
                let s = skip;
                skip = *l == "# wolos-tls";
                !(s || skip)
            })
            .map(|l| format!("{l}\n"))
            .collect()
    };
    assert_eq!(
        without.trim_end(),
        ARCH_OPENSSL_CNF.trim_end(),
        "nothing else changed"
    );
    // GnuTLS: the package's lines first, then the override.
    assert!(g.starts_with(ARCH_GNUTLS_CONFIG), "{g}");
    assert_eq!(
        &g[ARCH_GNUTLS_CONFIG.len()..],
        format!(
            "# wolos-tls\n[overrides]\n# wolos-tls\ndefault-priority-string = {}\n",
            net::GNUTLS_PRIORITY
        )
    );
}

/// Before wolos-system goes, the Remove hook runs `tls-policy --remove`: both package files are
/// as they were before the policy, so OpenSSL never includes a file that is gone.
#[test]
fn removing_the_policy_gives_the_package_files_back() {
    let h = read("files/usr/share/libalpm/hooks/wolos-tls-remove.hook");
    for want in [
        "Type = Path",
        "Operation = Remove",
        "Target = usr/lib/wolos/tls-policy",
        "When = PreTransaction",
        "Exec = /usr/lib/wolos/tls-policy --remove",
    ] {
        assert!(h.lines().any(|l| l == want), "{want} missing:\n{h}");
    }
    let r = root("remove", ARCH_OPENSSL_CNF, ARCH_GNUTLS_CONFIG);
    assert_eq!(apply(&r.0).0, 0);
    assert!(
        text(&r.0, "etc/ssl/openssl.cnf").contains(".include /usr/share/wolos/tls/openssl.cnf")
    );
    for _ in 0..2 {
        assert_eq!(run(&r.0, &["--remove"]), (0, String::new()));
        assert_eq!(text(&r.0, "etc/ssl/openssl.cnf"), ARCH_OPENSSL_CNF);
        assert_eq!(text(&r.0, "etc/gnutls/config"), ARCH_GNUTLS_CONFIG);
    }
    assert_eq!(run(&r.0, &["--bogus"]).0, 2, "unknown argument");
}

/// A changed policy replaces the old lines (the hook runs after an upgrade of wolos-system).
#[test]
fn a_new_policy_replaces_the_old_lines() {
    let r = root("new", ARCH_OPENSSL_CNF, ARCH_GNUTLS_CONFIG);
    assert_eq!(apply(&r.0).0, 0);
    let p = r.0.join("usr/share/wolos/tls/gnutls.priority");
    fs::write(&p, "# new\nNORMAL:-VERS-ALL:+VERS-TLS1.3\n").unwrap();
    assert_eq!(apply(&r.0).0, 0);
    let g = text(&r.0, "etc/gnutls/config");
    assert_eq!(g.matches("default-priority-string").count(), 1, "{g}");
    assert!(
        g.ends_with("default-priority-string = NORMAL:-VERS-ALL:+VERS-TLS1.3\n"),
        "{g}"
    );
}

/// Somebody else's `ssl_conf` or priority string is kept, and the script says it did not apply.
#[test]
fn an_existing_setting_is_left_alone_and_said() {
    let own_o = format!("{ARCH_OPENSSL_CNF}\n[openssl_init]\nssl_conf = mine\n");
    let own_g = format!("{ARCH_GNUTLS_CONFIG}[overrides]\ndefault-priority-string = SECURE256\n");
    let r = root("own", &own_o, &own_g);
    let (rc, err) = apply(&r.0);
    assert_eq!(rc, 1);
    assert!(
        err.contains("ssl_conf of its own") && err.contains("default-priority-string of its own"),
        "{err}"
    );
    assert_eq!(text(&r.0, "etc/ssl/openssl.cnf"), own_o);
    assert_eq!(text(&r.0, "etc/gnutls/config"), own_g);
}

/// No flag file the browser or a web runtime reads turns off ECH, GREASE or certificate checks.
#[test]
fn no_browser_flag_weakens_tls() {
    let bad = [
        "--disable-ech",
        "--disable-grease-tls",
        "--ignore-certificate-errors",
        "--ssl-version-min",
        "--ssl-key-log-file",
        "--disable-features=EncryptedClientHello",
        "PostQuantumKyber",
    ];
    let mut files = Vec::new();
    let base = repo::repo("desktop/overlay/usr/share/wolos");
    let mut stack = vec![base];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.to_string_lossy().ends_with("-flags.conf") {
                files.push(p);
            }
        }
    }
    assert!(
        files.iter().any(|p| p.ends_with("helium-flags.conf")),
        "found the flag files"
    );
    for f in files {
        let t = fs::read_to_string(&f).unwrap();
        for b in bad {
            assert!(!t.contains(b), "{}: {b}", f.display());
        }
    }
}

/// A pristine Arch container (the snapshot's openssl, gnutls, curl), the policy files copied in
/// and `tls-policy` run by the pacman hook: OpenSSL, curl and GnuTLS each negotiate the hybrid
/// group with a post-quantum server, TLS 1.1 and CBC suites are refused. `WOLOS_CONTAINER` picks
/// the runtime (default: podman, else docker).
#[test]
#[ignore = "needs docker or podman, the archlinux image and the network"]
fn a_pristine_arch_negotiates_the_hybrid_group() {
    let rt = std::env::var("WOLOS_CONTAINER").unwrap_or_else(|_| {
        if Command::new("podman").arg("--version").output().is_ok() {
            "podman"
        } else {
            "docker"
        }
        .into()
    });
    let files = repo::repo("files");
    let host = net::PROBE_HOSTS[0];
    let script = format!(
        "set -e; pacman -Sy --noconfirm --needed openssl gnutls curl >/dev/null; \
         install -d /usr/share/wolos/tls /usr/lib/wolos /usr/share/libalpm/hooks; \
         cp /f/usr/share/wolos/tls/* /usr/share/wolos/tls/; install -m755 /f/usr/lib/wolos/tls-policy /usr/lib/wolos/; \
         cp /f/usr/share/libalpm/hooks/wolos-tls.hook /usr/share/libalpm/hooks/; \
         pacman -S --noconfirm openssl gnutls >/dev/null; \
         echo '--openssl'; echo | openssl s_client -connect {host}:443 -servername {host} -brief 2>&1; \
         echo '--curl'; curl -sv --disable -o /dev/null https://{host}/ 2>&1; \
         echo '--gnutls'; gnutls-cli {host} </dev/null 2>&1 | grep -- '- Description'; \
         echo '--tls11'; echo | openssl s_client -connect {host}:443 -tls1_1 -brief 2>&1 || true; \
         echo '--ciphers'; openssl ciphers -s -tls1_2; echo '--end'"
    );
    let o = Command::new(&rt)
        .args(["run", "--rm", "-v"])
        .arg(format!("{}:/f:ro", files.display()))
        .args(["docker.io/library/archlinux:latest", "bash", "-c", &script])
        .output()
        .expect("container runtime");
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        o.status.success(),
        "{out}\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let part = |a: &str, b: &str| {
        let s = out.find(a).unwrap() + a.len();
        out[s..s + out[s..].find(b).unwrap()].to_string()
    };
    let ossl = net::parse_openssl(&part("--openssl", "--curl")).unwrap();
    let curl = net::parse_curl(&part("--curl", "--gnutls")).unwrap();
    let gtls = net::parse_gnutls(&part("--gnutls", "--tls11")).unwrap();
    eprintln!("openssl {ossl:?}\ncurl {curl:?}\ngnutls {gtls:?}");
    for n in [&ossl, &curl, &gtls] {
        assert!(n.post_quantum(), "{n:?}");
        assert_eq!(n.version, "TLSv1.3");
    }
    assert!(
        part("--tls11", "--ciphers").contains("no protocols available"),
        "TLS 1.1 refused"
    );
    for c in part("--ciphers", "--end").trim().split(':') {
        assert!(
            c.starts_with("ECDHE-") && (c.contains("GCM") || c.contains("CHACHA20")),
            "{c}"
        );
    }
}
