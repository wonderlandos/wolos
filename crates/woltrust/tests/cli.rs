// SPDX-License-Identifier: BSD-2-Clause
//! The `woltrust` command line with the flags of the hub-index "Tooling contract": `sign` prints a
//! `.wsig` that verifies, `issue` signs exactly the body it was given and checks the proof,
//! `revoke` prints a list the crate parses, `verify` prints the I7 report and exits 0 only for a
//! trusted file, and key material only ever comes from the environment or a file.
#![cfg(feature = "seal")]
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use woltrust::cert::{Kind, Level, Subject, Usage};
use woltrust::key::KeyMaterial;
use woltrust::pki::{self, DAY, NOW, Pki, Spec};
use woltrust::{Cert, Purpose, Wsig, b64, crypto, make, request};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("woltrust-cli-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn run(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_woltrust"));
    c.args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("woltrust runs")
}

fn km_text(name: &str) -> String {
    let km = KeyMaterial::new(pki::seed(name), name.starts_with("test-root"));
    String::from_utf8(km.to_text().to_vec()).expect("utf-8")
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let p = dir.join(name);
    std::fs::write(&p, bytes).expect("write");
    p.to_string_lossy().into_owned()
}

#[test]
fn sign_prints_a_wsig_that_verifies_with_the_index_key() {
    let d = tmp("sign");
    let p = Pki::new();
    let file = write(&d, "index.json", b"{\"format\":1}");
    let out = run(
        &[
            "sign",
            "--purpose",
            "index",
            "--cert",
            &p.index.body.serial,
            "--key-env",
            "IDX",
            &file,
        ],
        &[("IDX", &km_text("test-index"))],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let w = Wsig::parse(&out.stdout).expect("a .wsig");
    assert_eq!(w.cert, p.index.body.serial);
    let msg = crypto::message(Purpose::Index, b"{\"format\":1}");
    let sig = w.sig().expect("halves");
    assert!(crypto::verify_hybrid(&p.index.cert().expect("cert").keys, &msg, &sig).is_ok());
    let missing = run(
        &[
            "sign",
            "--purpose",
            "index",
            "--cert",
            &p.index.body.serial,
            "--key-env",
            "NOPE",
            &file,
        ],
        &[],
    );
    assert_eq!(missing.status.code(), Some(1));
    let usage = run(&["sign", "--purpose", "nothing", "--cert", "x", &file], &[]);
    assert_eq!(usage.status.code(), Some(2));
}

#[test]
fn issue_signs_exactly_the_body_and_needs_a_valid_proof() {
    let d = tmp("issue");
    let p = Pki::new();
    let author = KeyMaterial::new(pki::seed("cli-author"), false);
    let spec = Spec {
        name: "cli-author",
        kind: Kind::Person,
        subject: Subject {
            github_id: Some(77),
            login: Some("carol".into()),
            name: Some("Carol".into()),
            label: None,
            org: None,
            text: None,
        },
        namespaces: &["carol."],
        usage: &[Usage::SignPackage],
        issuer: p.authors.body.serial.clone(),
        not_before: NOW - DAY,
        not_after: NOW + 300 * DAY,
        level: Level::Verified,
    };
    let mut body = pki::body(&spec, &author.secret());
    body.keys = author.keys_text();
    let body_bytes = serde_json::to_vec(&body).expect("json");
    let body_file = write(&d, "body.json", &body_bytes);
    let proof = request::prove(
        &author.secret(),
        &request::Claimed::of_body(&body).expect("claims"),
    );
    let proof_file = write(&d, "proof.json", &serde_json::to_vec(&proof).expect("json"));
    let key_file = write(&d, "ca.key", km_text("test-authors").as_bytes());
    let out = run(
        &[
            "issue",
            "--body",
            &body_file,
            "--proof",
            &proof_file,
            "--key-file",
            &key_file,
        ],
        &[],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let c = Cert::parse(&out.stdout).expect("a .wolcert");
    assert_eq!(
        c.body_bytes, body_bytes,
        "the body is exactly the bytes asked for"
    );
    assert!(c.enc.is_some());
    let msg = crypto::message(Purpose::Cert, &body_bytes);
    assert!(crypto::verify_hybrid(&p.authors.cert().expect("ca").keys, &msg, &c.sig).is_ok());
    let none = run(
        &["issue", "--body", &body_file, "--key-file", &key_file],
        &[],
    );
    assert_eq!(none.status.code(), Some(1), "a person needs --proof");
    let stolen = request::prove(
        &KeyMaterial::new([1; 32], false).secret(),
        &request::Claimed::of_body(&body).expect("claims"),
    );
    let bad = write(&d, "bad.json", &serde_json::to_vec(&stolen).expect("json"));
    let refused = run(
        &[
            "issue",
            "--body",
            &body_file,
            "--proof",
            &bad,
            "--key-file",
            &key_file,
        ],
        &[],
    );
    assert_eq!(refused.status.code(), Some(1));
    // Audit H2: the CA asked to sign a body with a namespace the requester did not ask for (the
    // proof is the requester's own): refused, nothing printed.
    let mut wider = body.clone();
    wider.namespaces.push("mozilla.".into());
    let wider_file = write(&d, "wider.json", &serde_json::to_vec(&wider).expect("json"));
    let squat = run(
        &[
            "issue",
            "--body",
            &wider_file,
            "--proof",
            &proof_file,
            "--key-file",
            &key_file,
        ],
        &[],
    );
    assert_eq!(squat.status.code(), Some(1));
    assert!(squat.stdout.is_empty());
}

#[test]
fn revoke_builds_the_next_list_and_verify_reports_it() {
    let d = tmp("revoke");
    let p = Pki::new();
    let src = format!(
        r#"{{"format":1,"entries":[{{"serial":"{}","at":{},"reason":"key-compromise"}}]}}"#,
        p.alice.body.serial,
        NOW - DAY
    );
    let src = write(&d, "revocations.json", src.as_bytes());
    let now = NOW.to_string();
    let out = run(&["revoke", "--source", &src, "--now", &now], &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let l = woltrust::revoked::parse(&out.stdout).expect("a list");
    assert_eq!((l.serial, l.entries.len()), (1, 1));
    let list = write(&d, "revoked.json", &out.stdout);
    let sig = make::wsig(
        &p.index.body.serial,
        &p.index.secret,
        Purpose::Revocation,
        &out.stdout,
    );
    write(&d, "revoked.json.wsig", &sig);
    let next = run(
        &[
            "revoke",
            "--source",
            &src,
            "--previous",
            &list,
            "--now",
            &now,
        ],
        &[],
    );
    assert_eq!(
        woltrust::revoked::parse(&next.stdout).expect("list").serial,
        2
    );
    let roots = tmp("revoke-roots");
    write(&roots, "a.wolcert", &p.root_a.file);
    let inter = tmp("revoke-inter");
    write(&inter, "index.wolcert", &p.index.file);
    write(&inter, "authors.wolcert", &p.authors.file);
    let content = write(&d, "SHA256", b"manifest");
    let wsig = write(
        &d,
        "SHA256.wsig",
        &make::wsig(
            &p.alice.body.serial,
            &p.alice.secret,
            Purpose::Package,
            b"manifest",
        ),
    );
    let chain = write(&d, "CHAIN", &Pki::chain(&[&p.alice]));
    let base = |list_arg: bool| {
        let mut a = vec![
            "verify",
            "--purpose",
            "package",
            "--wsig",
            &wsig,
            "--chain",
            &chain,
            "--id",
            "alice.notes",
            "--now",
            &now,
            "--allow-test",
        ];
        let r = roots.to_string_lossy().into_owned();
        let i = inter.to_string_lossy().into_owned();
        let mut owned = vec!["--roots".to_owned(), r, "--intermediates".to_owned(), i];
        if list_arg {
            owned.extend(["--list".to_owned(), list.clone()]);
        }
        owned.push(content.clone());
        a.extend(owned.iter().map(String::as_str));
        run(&a, &[])
    };
    let withdrawn = base(true);
    assert_eq!(withdrawn.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&withdrawn.stdout).expect("I7 json");
    assert_eq!(report["trust"], "withdrawn");
    let unlisted = base(false);
    let report: serde_json::Value = serde_json::from_slice(&unlisted.stdout).expect("I7 json");
    // Audit M3: with no list the signature keeps its value, flagged as not checked.
    assert_eq!(report["trust"], "origin", "no list: still the author's");
    assert!(
        unlisted.status.success(),
        "a verified signature exits 0 without a list"
    );
    let f = report["findings"][0]["rule"].as_str().unwrap_or_default();
    assert_eq!(f, "trust/list-too-old");
}

#[test]
fn keygen_and_public_give_matching_keys() {
    let out = run(&["keygen"], &[]);
    assert!(out.status.success());
    let km = KeyMaterial::parse(&out.stdout).expect("key material");
    let text = String::from_utf8(out.stdout).expect("utf-8");
    let public = run(&["public", "--key-env", "K"], &[("K", &text)]);
    let keys: woltrust::cert::KeysText = serde_json::from_slice(&public.stdout).expect("keys");
    assert_eq!(keys.ed25519, b64::encode(&km.secret().public().ed25519));
    let root = run(&["keygen", "--root"], &[]);
    assert!(KeyMaterial::parse(&root.stdout).expect("root key").root);
}

#[test]
fn init_writes_locked_keys_that_sign_from_the_environment() {
    let d = tmp("init");
    let out_dir = d.join("authority");
    let dir = out_dir.to_string_lossy().into_owned();
    // The subject text is required, with no default.
    let none = run(
        &["init", "--out", &dir, "--passphrase-env", "P"],
        &[("P", "lead passphrase")],
    );
    assert!(!none.status.success() && !out_dir.exists());
    let out = run(
        &[
            "init",
            "--out",
            &dir,
            "--subject",
            "WolOS test authority - example.org",
            "--passphrase-env",
            "P",
        ],
        &[("P", "lead passphrase")],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert_eq!(text.lines().count(), 5, "five plain lines: {text}");
    for e in std::fs::read_dir(out_dir.join("public/roots"))
        .expect("roots")
        .flatten()
    {
        let c = woltrust::Cert::parse(&std::fs::read(e.path()).expect("root")).expect("parses");
        assert_eq!(
            c.body.subject.text.as_deref(),
            Some("WolOS test authority - example.org"),
            "the subject text is signed in each root"
        );
    }
    let roots = std::fs::read_dir(out_dir.join("public/roots"))
        .expect("roots")
        .count();
    let inter = std::fs::read_dir(out_dir.join("public/intermediates"))
        .expect("inter")
        .count();
    assert_eq!((roots, inter), (2, 3));
    let index_key = std::fs::read_to_string(out_dir.join("ci/index.wolkey")).expect("key");
    let ci_pass = std::fs::read_to_string(out_dir.join("ci/passphrase")).expect("pass");
    let serial = woltrust::wolkey::name_of(index_key.as_bytes()).expect("name");
    let file = write(&d, "index.json", b"{}");
    let signed = run(
        &[
            "sign",
            "--purpose",
            "index",
            "--cert",
            &serial,
            "--key-env",
            "K",
            "--passphrase-env",
            "Q",
            &file,
        ],
        &[("K", &index_key), ("Q", &ci_pass)],
    );
    assert!(
        signed.status.success(),
        "{}",
        String::from_utf8_lossy(&signed.stderr)
    );
    let wrong = run(
        &[
            "sign",
            "--purpose",
            "index",
            "--cert",
            &serial,
            "--key-env",
            "K",
            "--passphrase-env",
            "Q",
            &file,
        ],
        &[("K", &index_key), ("Q", "lead passphrase")],
    );
    assert_eq!(
        wrong.status.code(),
        Some(1),
        "the CI key opens only with the CI passphrase"
    );
    let backup = out_dir
        .join("wolos-trust.wolbackup")
        .to_string_lossy()
        .into_owned();
    let root = run(&["extract", "--backup", &backup, "--role", "root-a"], &[]);
    assert!(woltrust::wolkey::name_of(&root.stdout).is_ok());
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&backup)
        .expect("meta")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let again = run(
        &[
            "init",
            "--out",
            &dir,
            "--subject",
            "WolOS test authority",
            "--passphrase-env",
            "P",
        ],
        &[("P", "lead passphrase")],
    );
    assert_eq!(again.status.code(), Some(1), "never overwrites");
}
