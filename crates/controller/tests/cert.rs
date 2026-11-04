// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol cert`, `wol sign`, `wol encrypt --to`, `wol decrypt`, `wol trust status` (ADR 0046): keys
//! are made and kept locked under the data home, a certificate from the TEST Authors CA is added,
//! a file sealed to it opens again, a `.wol` signed with it verifies as a verified author, and a
//! wrong passphrase opens nothing.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use std::path::{Path, PathBuf};
use wol_core::woltrust::cert::{Body, Level, Usage};
use wol_core::woltrust::pki::{DAY, NOW, Pki};
use wol_core::woltrust::{Code, Store, Trust, make, request};

fn run(args: &[&str], env: &mut FakeEnv) -> i32 {
    execute(&parse(args).expect("parses"), env)
}

fn home(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-cert-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp");
    d
}

fn env_at(d: &Path) -> FakeEnv {
    FakeEnv {
        vars: vec![("XDG_DATA_HOME".into(), d.to_string_lossy().into_owned())],
        ..FakeEnv::default()
    }
}

/// What the hub's ca workflow does with the request: the TEST Authors CA issues it.
fn issue(p: &Pki, req: &request::Request) -> Vec<u8> {
    let body = Body {
        serial: "00c0ffee00c0ffee".into(),
        kind: req.kind,
        subject: req.subject.clone(),
        namespaces: req.namespaces.clone(),
        usage: vec![Usage::SignPackage, Usage::SignPlugin, Usage::SignExtension],
        keys: req.keys.clone(),
        issuer: p.authors.body.serial.clone(),
        not_before: 0,
        not_after: NOW + 300 * DAY,
        level: Level::Verified,
        verified_at: None,
        class: None,
        origin_cert: None,
    };
    make::issue(&body, &p.authors.secret)
}

#[test]
fn request_add_seal_open_and_sign_work_end_to_end() {
    let d = home("flow");
    let p = Pki::new();
    let mut env = env_at(&d);
    env.stdin = vec!["correct horse battery".into()];
    let rc = run(
        &["cert", "request", "1001", "alice", "alice.", "Alice", "Doe"],
        &mut env,
    );
    assert_eq!(rc, 0, "{}", env.err);
    let req: request::Request = serde_json::from_str(&env.out).expect("request json");
    assert!(
        req.keys.ml_kem_768.is_some(),
        "encryption keys in the request"
    );
    let keys_dir = d.join("wolos/trust/keys");
    let key_files: Vec<_> = std::fs::read_dir(&keys_dir)
        .expect("keys")
        .flatten()
        .collect();
    assert_eq!(key_files.len(), 1);
    use std::os::unix::fs::PermissionsExt;
    let meta = key_files[0].metadata().expect("meta");
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let text = std::fs::read_to_string(key_files[0].path()).expect("key text");
    assert!(!text.contains("seed"), "never written in the clear");

    let cert = d.join("alice.wolcert");
    std::fs::write(&cert, issue(&p, &req)).expect("cert");
    let mut env = env_at(&d);
    env.stdin = vec![
        "correct horse battery".into(),
        "correct horse battery".into(),
    ];
    assert_eq!(
        run(&["cert", "add", &cert.to_string_lossy()], &mut env),
        0,
        "{}",
        env.err
    );
    assert!(keys_dir.join("00c0ffee00c0ffee.wolkey").exists());
    let mut env = env_at(&d);
    assert_eq!(run(&["cert", "list"], &mut env), 0);
    assert!(
        env.out.contains("00c0ffee00c0ffee") && env.out.contains("(your key)"),
        "{}",
        env.out
    );

    let secret = d.join("note.txt");
    std::fs::write(&secret, b"meet at noon").expect("plain");
    let mut env = env_at(&d);
    let f = secret.to_string_lossy().into_owned();
    assert_eq!(
        run(&["encrypt", &f, "--to", "00c0ffee00c0ffee"], &mut env),
        0,
        "{}",
        env.err
    );
    std::fs::remove_file(&secret).expect("rm");
    let sealed = format!("{f}.wolsealed");
    let mut env = env_at(&d);
    env.stdin = vec!["wrong passphrase".into()];
    assert_ne!(run(&["decrypt", &sealed], &mut env), 0);
    assert!(!secret.exists(), "nothing written on a wrong passphrase");
    let mut env = env_at(&d);
    env.stdin = vec!["correct horse battery".into()];
    assert_eq!(run(&["decrypt", &sealed], &mut env), 0, "{}", env.err);
    assert_eq!(std::fs::read(&secret).expect("opened"), b"meet at noon");

    let info = r#"{"id":"alice.notes","version":"1.0","package":"alice-notes","targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/n"}]}"#;
    let man = wol_core::package::manifest(&[("Info.json", info.as_bytes()), ("payload.pkg", b"x")]);
    let wol = d.join("notes.wol");
    std::fs::write(
        &wol,
        wol_core::package::ustar(&[
            ("Info.json", info.as_bytes()),
            ("payload.pkg", b"x"),
            ("SHA256", man.as_bytes()),
        ]),
    )
    .expect("wol");
    let w = wol.to_string_lossy().into_owned();
    let repo = "https://github.com/alice/notes";
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let mut env = env_at(&d);
    env.stdin = vec!["correct horse battery".into()];
    let theirs = [
        "sign",
        w.as_str(),
        "--as",
        "00c0ffee00c0ffee",
        "https://github.com/bob/notes",
        "v1.0",
        commit,
    ];
    assert_ne!(
        run(&theirs, &mut env),
        0,
        "someone else's repository is refused"
    );
    let mut env = env_at(&d);
    env.stdin = vec!["correct horse battery".into()];
    let rc = run(
        &["sign", &w, "--as", "00c0ffee00c0ffee", repo, "v1.0", commit],
        &mut env,
    );
    assert_eq!(rc, 0, "{}", env.err);
    let l = wol_core::woltrust::revoked::ListText {
        format: 1,
        serial: 1,
        issued: NOW - DAY,
        expires: NOW - DAY + wol_core::woltrust::LIST_LIFETIME,
        entries: vec![],
    };
    let lists = [make::revocation_list(
        &l,
        &p.index.body.serial,
        &p.index.secret,
    )];
    let ts = wol_core::trust::TrustStore {
        store: Store::load(
            &[&p.root_a.file],
            &[&p.authors.file, &p.index.file],
            &lists,
            NOW,
            true,
        ),
    };
    let c = ts.verify_wol_at(&wol, &[], None, NOW).expect("structure");
    assert_eq!((c.outcome.trust, c.outcome.code), (Trust::Origin, Code::Ok));
    sign_wolplug(&d, &ts);
}

/// A `.wolplug` (zstd of the ustar) for `id`, unsigned, as `wol plugin pack` makes it; None
/// without zstd on this machine.
fn wolplug(d: &Path, id: &str) -> Option<PathBuf> {
    let files: Vec<(String, Vec<u8>)> =
        wol_core::plugins::scaffold(id, "Demo", &[shared::plugins::Kind::Theme])
            .expect("scaffold")
            .into_iter()
            .map(|(r, t)| (format!("{id}/{r}"), t.into_bytes()))
            .collect();
    let mut all: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let sha = wol_core::package::manifest(&all);
    all.push(("SHA256", sha.as_bytes()));
    let tar = d.join(format!("{id}.tar"));
    std::fs::write(&tar, wol_core::package::ustar(&all)).expect("tar");
    let out = d.join(format!("{id}-0.1.wolplug"));
    let ok = std::process::Command::new("zstd")
        .args(["-q", "-f", "--rm", "-o"])
        .arg(&out)
        .arg(&tar)
        .status()
        .is_ok_and(|s| s.success());
    ok.then_some(out)
}

/// `wol sign` on a `.wolplug`: the plugin comes back as a zstd file whose `CHAIN` and
/// `SHA256.wsig` (label `wolos/plugin/1`) verify as Alice's; a plugin outside her names is refused
/// before any passphrase is asked.
fn sign_wolplug(d: &Path, ts: &wol_core::trust::TrustStore) {
    let Some(mine) = wolplug(d, "alice.demo") else {
        eprintln!("no zstd here: the .wolplug signing half is skipped");
        return;
    };
    let f = mine.to_string_lossy().into_owned();
    let mut env = env_at(d);
    env.stdin = vec!["correct horse battery".into()];
    assert_eq!(
        run(&["sign", &f, "--as", "00c0ffee00c0ffee"], &mut env),
        0,
        "{}",
        env.err
    );
    let tar = controller::plugin::wolplug_tar(&mine).expect("still a zstd .wolplug");
    let c = ts.verify_wolplug(&tar, &[], NOW).expect("verifies");
    assert_eq!((c.outcome.trust, c.outcome.code), (Trust::Origin, Code::Ok));
    assert_eq!(c.verified.manifest.id, "alice.demo");
    let mut env = env_at(d);
    env.stdin = vec!["correct horse battery".into()];
    assert_ne!(run(&["sign", &f, "--as", "00c0ffee00c0ffee"], &mut env), 0);
    assert!(env.err.contains("already"), "{}", env.err);
    let theirs = wolplug(d, "bob.demo").expect("zstd ran once");
    let mut env = env_at(d);
    let t = theirs.to_string_lossy().into_owned();
    assert_ne!(run(&["sign", &t, "--as", "00c0ffee00c0ffee"], &mut env), 0);
    assert!(env.err.contains("outside"), "{}", env.err);
}

#[test]
fn trust_status_and_usage_errors_say_what_to_do() {
    let d = home("status");
    let mut env = env_at(&d);
    assert_eq!(run(&["trust", "status"], &mut env), 0);
    assert!(env.out.contains("roots:"), "{}", env.out);
    let mut env = env_at(&d);
    assert_ne!(run(&["encrypt", "a.txt", "0123456789abcdef"], &mut env), 0);
    assert!(env.err.contains("--to"), "{}", env.err);
    let mut env = env_at(&d);
    assert_ne!(run(&["cert", "show", "0123456789abcdef"], &mut env), 0);
    // The disk-encryption words still parse as before.
    assert_eq!(
        parse(&["encrypt", "rebind"]).expect("parses").action,
        "encrypt.rebind"
    );
}
