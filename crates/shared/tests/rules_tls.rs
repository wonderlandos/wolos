// SPDX-License-Identifier: BSD-2-Clause
//! ADR 0055: one TLS setup for WolOS's own Rust programs. Every TLS client config, provider and
//! ureq agent comes from `wol_core::net` (`tls_provider`, `tls_config`, `agent_config`: the hybrid
//! post-quantum group first, AEAD only), so a crate cannot quietly go back to a classical-only
//! default. Scanned in every `.rs` under `crates/` with comments and strings blanked by the shared
//! lexer, after `use` items are resolved (an alias `use rustls::ClientConfig as C;` makes `C::builder`
//! read `rustls::ClientConfig::builder`; a glob import of a TLS crate is a hit by itself). Files under
//! a `tests/` directory are test code and may build their own (a live handshake with one group).
//! Every `Cargo.toml` is scanned for a second TLS library.
#[path = "support/lexer.rs"]
mod lexer;
use lexer::is_ident;
use std::path::{Path, PathBuf};

/// Where a TLS config, provider or agent may be built, and why.
const ALLOWED: &[(&str, &str)] = &[
    (
        "crates/core/src/net.rs",
        "the one place (tls_provider, tls_config, agent_config, probe)",
    ),
    // The web runtime engine's own rustls (aws-lc-rs, in the engine's workspace): the shim reorders
    // that provider's groups the same way (src/engine/tls.rs).
    (
        "crates/webengine/src/engine/lib.rs",
        "the engine's provider, hybrid first",
    ),
    // Decided (CTO, PQ-TLS review): federate.network's door pins its ISRG roots and puts
    // wolcrypt::tls's hybrid group first on ring's provider itself (ADR 0052 "Transport").
    (
        "crates/federate/src/source.rs",
        "the fed:// door: pinned roots, hybrid group first",
    ),
    // ring's generator for random bytes of the sealed state, no TLS config.
    (
        "crates/federate/src/state.rs",
        "random bytes from ring's provider, no TLS",
    ),
];

/// What building a TLS client config, a provider or a ureq agent looks like, as resolved paths
/// (each matches the path itself and any longer path ending in it).
const BUILDS: &[&str] = &[
    // rustls / ureq TLS configs
    "TlsConfig::builder",
    "ClientConfig::builder",
    "builder_with_provider",
    "rustls_crypto_provider",
    // providers
    "default_provider",
    "CryptoProvider {",
    "install_default",
    "kx_groups",
    "cipher_suites",
    // ureq agents and agent-less requests (ureq's own default config)
    "Agent::config_builder",
    "Agent::new_with_defaults",
    "Agent::new_with_config",
    "ureq::config::Config::builder",
    "ureq::Config::builder",
    "ureq::agent(",
    "ureq::get(",
    "ureq::post(",
    "ureq::put(",
    "ureq::delete(",
    "ureq::head(",
    "ureq::patch(",
    "ureq::options(",
    "ureq::trace(",
    "ureq::connect(",
    "ureq::run(",
    // other stacks
    "native_tls::",
    "openssl::ssl",
    // glob imports of a TLS crate hide every name above
    "use rustls::*",
    "use ureq::*",
    "use rustls::crypto::*",
];

/// TLS libraries no WolOS crate links (a second stack, or a C one beside rustls).
const OTHER_TLS: &[&str] = &[
    "native-tls",
    "openssl",
    "openssl-sys",
    "reqwest",
    "boring",
    "aws-lc-rs",
];

fn root() -> PathBuf {
    let r = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    r.canonicalize().unwrap_or(r)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, name: &dyn Fn(&str) -> bool) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let n = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !matches!(
                n.as_str(),
                "target" | "fixtures" | "node_modules" | ".git" | "tests"
            ) {
                walk(&p, out, name);
            }
        } else if name(&n) {
            out.push(p);
        }
    }
}

fn rel(p: &Path) -> String {
    p.strip_prefix(root())
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// `a::b::{c, d as e, f::{self, g}}` -> [(a::b::c, c), (a::b::d, e), (a::b::f, f), (a::b::f::g, g)].
fn expand(prefix: &str, tree: &str, out: &mut Vec<(String, String)>) {
    let tree = tree.trim();
    if let Some(open) = tree.find('{') {
        let head = tree[..open].trim().trim_end_matches("::");
        let inner = tree[open + 1..].trim_end().trim_end_matches('}');
        let base = join(prefix, head);
        let mut depth = 0;
        let mut start = 0;
        for (i, c) in inner.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    expand(&base, &inner[start..i], out);
                    start = i + 1;
                }
                _ => {}
            }
        }
        expand(&base, &inner[start..], out);
        return;
    }
    if tree.is_empty() {
        return;
    }
    let (path, alias) = match tree.split_once(" as ") {
        Some((p, a)) => (p.trim(), Some(a.trim())),
        None => (tree, None),
    };
    if path == "self" {
        let last = prefix.rsplit("::").next().unwrap_or(prefix);
        out.push((prefix.to_string(), alias.unwrap_or(last).to_string()));
        return;
    }
    let full = join(prefix, path);
    let last = full.rsplit("::").next().unwrap_or(&full).to_string();
    if last != "*" && alias != Some("_") {
        out.push((full.clone(), alias.unwrap_or(&last).to_string()));
    }
}

fn join(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a}::{b}"),
    }
}

/// `text`'s code (comments and strings blanked), each `use` item kept (whitespace squeezed) and
/// every other path's first segment replaced by what a `use` made it name.
fn resolved(text: &str) -> String {
    let (code, _) = lexer::lex(text);
    let code = String::from_utf8_lossy(&code).into_owned();
    // `use` items: (start, end) byte ranges and the names they bring in.
    let mut names: Vec<(String, String)> = Vec::new();
    let mut uses: Vec<(usize, usize, String)> = Vec::new();
    let b = code.as_bytes();
    let mut i = 0;
    while let Some(off) = code[i..].find("use ") {
        let at = i + off;
        let word_start = at == 0 || !is_ident(b[at - 1]);
        let end = code[at..].find(';').map(|e| at + e).unwrap_or(code.len());
        if word_start {
            let item: String = code[at + 4..end]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let item = item
                .trim_start_matches("::")
                .replace(" ::", "::")
                .replace(":: ", "::");
            expand("", &item, &mut names);
            uses.push((at, end, format!("use {item}")));
        }
        i = end.max(at + 4).min(code.len());
        if i >= code.len() {
            break;
        }
    }
    // Rebuild: use items as squeezed text, identifiers that start a path replaced by their full path.
    let mut out = String::with_capacity(code.len());
    let mut p = 0;
    let mut u = uses.iter().peekable();
    while p < b.len() {
        if let Some((s, e, item)) = u.peek()
            && *s == p
        {
            out.push_str(item);
            out.push(';');
            p = (*e + 1).min(b.len());
            u.next();
            continue;
        }
        if is_ident(b[p]) && (p == 0 || (!is_ident(b[p - 1]) && !code[..p].ends_with("::"))) {
            let mut q = p;
            while q < b.len() && is_ident(b[q]) {
                q += 1;
            }
            let word = &code[p..q];
            match names.iter().find(|(_, n)| n == word) {
                Some((full, _)) => out.push_str(full),
                None => out.push_str(word),
            }
            p = q;
            continue;
        }
        out.push(b[p] as char);
        p += 1;
    }
    out
}

/// The hits of [`BUILDS`] in `text`'s resolved code.
fn builds(text: &str) -> Vec<&'static str> {
    let code = resolved(text);
    let squeezed: String = code.split_whitespace().collect::<Vec<_>>().join(" ");
    let tight = squeezed
        .replace(" (", "(")
        .replace(":: ", "::")
        .replace(" ::", "::");
    BUILDS
        .iter()
        .copied()
        .filter(|b| tight.contains(b))
        .collect()
}

#[test]
fn only_wol_core_net_builds_a_tls_config() {
    let mut files = Vec::new();
    walk(&root().join("crates"), &mut files, &|n| n.ends_with(".rs"));
    assert!(files.len() > 100, "found the sources ({})", files.len());
    let mut bad = Vec::new();
    for f in &files {
        let r = rel(f);
        if ALLOWED.iter().any(|(a, _)| *a == r) {
            continue;
        }
        let t = std::fs::read_to_string(f).unwrap_or_default();
        for b in builds(&t) {
            bad.push(format!("{r}: {b}"));
        }
    }
    assert!(
        bad.is_empty(),
        "build TLS and ureq agents through wol_core::net (tls_config, agent_config, tls_provider; ADR 0055):\n{}",
        bad.join("\n")
    );
    // The allowed places really are where something is built (a stale row fails).
    for (a, why) in ALLOWED {
        let t = std::fs::read_to_string(root().join(a)).unwrap_or_else(|e| panic!("{a}: {e}"));
        assert!(
            !builds(&t).is_empty(),
            "{a} ({why}) builds nothing any more: drop its row"
        );
    }
}

/// The scanner sees code and not words: a comment or a string naming a builder is not a hit.
#[test]
fn the_scan_reads_code_only() {
    assert_eq!(
        builds(
            "// TlsConfig::builder()\nlet s = \"ClientConfig::builder\"; let u = \"ureq::get\";"
        ),
        Vec::<&str>::new()
    );
    assert_eq!(
        builds("let c = TlsConfig::builder().build();"),
        ["TlsConfig::builder"]
    );
    // The helper itself is the way: no hit.
    assert_eq!(
        builds("let a = wol_core::net::agent_config().https_only(true).build();"),
        Vec::<&str>::new()
    );
}

/// Every way around the helper that the review named is a hit.
#[test]
fn every_bypass_is_caught() {
    let cases: &[(&str, &str)] = &[
        (
            "let a = ureq::Agent::config_builder().build();",
            "Agent::config_builder",
        ),
        (
            "use ureq::Agent;\nlet a = Agent::config_builder().build();",
            "Agent::config_builder",
        ),
        (
            "use ureq::Agent as A;\nlet a = A::new_with_defaults();",
            "Agent::new_with_defaults",
        ),
        (
            "let a = ureq::Agent::new_with_config(c);",
            "Agent::new_with_config",
        ),
        ("let r = ureq::get(url).call();", "ureq::get("),
        ("use ureq::post;\nlet r = post(url).send(b);", "ureq::post("),
        (
            "use ureq::{get as fetch};\nlet r = fetch(url).call();",
            "ureq::get(",
        ),
        ("let r = ureq :: head (url).call();", "ureq::head("),
        (
            "use rustls::ClientConfig as Cc;\nlet c = Cc::builder();",
            "ClientConfig::builder",
        ),
        (
            "use rustls::{self as r};\nlet c = r::ClientConfig::builder();",
            "ClientConfig::builder",
        ),
        (
            "use rustls::{ClientConfig as K, RootCertStore};\nlet c = K::builder_with_provider(p);",
            "builder_with_provider",
        ),
        (
            "let p = rustls::crypto::ring::default_provider();",
            "default_provider",
        ),
        (
            "use rustls::crypto::aws_lc_rs as a;\nlet p = a::default_provider();",
            "default_provider",
        ),
        (
            "let p = CryptoProvider { cipher_suites: v, kx_groups: k, ..x };",
            "CryptoProvider {",
        ),
        ("p.kx_groups.insert(0, g);", "kx_groups"),
        ("p.install_default().ok();", "install_default"),
        (
            "let t = TlsConfig::builder().unversioned_rustls_crypto_provider(p).build();",
            "rustls_crypto_provider",
        ),
        (
            "use ureq::config::Config;\nlet c = Config::builder().build();",
            "ureq::config::Config::builder",
        ),
        ("use ureq::*;\nlet r = get(u);", "use ureq::*"),
        ("use rustls::*;", "use rustls::*"),
    ];
    for (code, want) in cases {
        let got = builds(code);
        assert!(got.contains(want), "{code:?}: wanted {want}, got {got:?}");
    }
}

#[test]
fn no_second_tls_library_in_a_manifest() {
    let mut files = Vec::new();
    walk(&root().join("crates"), &mut files, &|n| n == "Cargo.toml");
    let mut bad = Vec::new();
    for f in &files {
        let r = rel(f);
        let t = std::fs::read_to_string(f).unwrap_or_default();
        for l in t.lines().map(str::trim).filter(|l| !l.starts_with('#')) {
            let key = l.split(['=', ' ', '.']).next().unwrap_or("");
            // The web runtime engine's manifest names the engine's own stack (its workspace).
            if OTHER_TLS.contains(&key) && r != "crates/webengine/engine/Cargo.toml" {
                bad.push(format!("{r}: {l}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "one TLS stack (rustls over ring, ADR 0055):\n{}",
        bad.join("\n")
    );
}
