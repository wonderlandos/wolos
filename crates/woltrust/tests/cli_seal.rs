// SPDX-License-Identifier: BSD-2-Clause
//! `woltrust seal` / `woltrust open`: a file sealed to the keys `woltrust public` prints opens with
//! the key it came from, from a file or stdin, and refuses another key, another label, a changed
//! byte and keys without an encryption half. Throwaway keys only.
#![cfg(feature = "seal")]
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("woltrust-seal-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn run(args: &[&str], stdin: &[u8]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_woltrust"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("woltrust runs");
    c.stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    c.wait_with_output().expect("woltrust ends")
}

/// A fresh key file and its public keys file in `d`.
fn key(d: &std::path::Path, name: &str) -> (String, String) {
    let k = run(&["keygen"], b"");
    assert!(k.status.success());
    let kf = d.join(format!("{name}.key"));
    std::fs::write(&kf, &k.stdout).expect("write key");
    let kf = kf.to_string_lossy().into_owned();
    let p = run(&["public", "--key-file", &kf], b"");
    assert!(p.status.success());
    let pf = d.join(format!("{name}.json"));
    std::fs::write(&pf, &p.stdout).expect("write public");
    (kf, pf.to_string_lossy().into_owned())
}

#[test]
fn sealed_to_public_keys_opens_with_the_key_only() {
    let d = tmp("roundtrip");
    let (k, p) = key(&d, "a");
    let (k2, _) = key(&d, "b");
    let msg = b"throwaway test secret\n\x00\xff";
    let s = run(&["seal", "--to", &p, "--label", "wolos/vault/t", "-"], msg);
    assert!(s.status.success(), "{}", String::from_utf8_lossy(&s.stderr));
    assert!(s.stdout.starts_with(b"wolos/seal/1\0"));
    assert!(!s.stdout.windows(msg.len()).any(|w| w == msg));
    let sf = d.join("s.sealed");
    std::fs::write(&sf, &s.stdout).expect("write sealed");
    let sf = sf.to_string_lossy().into_owned();

    let o = run(
        &["open", "--label", "wolos/vault/t", "--key-file", &k, &sf],
        b"",
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.stdout, msg);
    let o = run(
        &["open", "--label", "wolos/vault/t", "--key-file", &k, "-"],
        &s.stdout,
    );
    assert_eq!(o.stdout, msg);

    let other = run(
        &["open", "--label", "wolos/vault/t", "--key-file", &k2, &sf],
        b"",
    );
    assert_eq!(other.status.code(), Some(1));
    assert!(other.stdout.is_empty());
    let label = run(
        &["open", "--label", "wolos/vault/u", "--key-file", &k, &sf],
        b"",
    );
    assert_eq!(label.status.code(), Some(1));
    let mut bad = s.stdout.clone();
    if let Some(b) = bad.last_mut() {
        *b ^= 1;
    }
    let damaged = run(
        &["open", "--label", "wolos/vault/t", "--key-file", &k, "-"],
        &bad,
    );
    assert_eq!(damaged.status.code(), Some(1));
    assert!(damaged.stdout.is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn seal_refuses_keys_without_encryption_half_and_bad_usage() {
    let d = tmp("refuse");
    let (_, p) = key(&d, "a");
    let text = std::fs::read_to_string(&p).expect("public");
    let mut v: serde_json::Value = serde_json::from_str(&text).expect("json");
    v.as_object_mut().expect("object").remove("ml-kem-768");
    let half = d.join("half.json");
    std::fs::write(&half, v.to_string()).expect("write");
    let r = run(&["seal", "--to", &half.to_string_lossy(), "-"], b"x");
    assert_eq!(r.status.code(), Some(1));
    let r = run(&["seal", "-"], b"x");
    assert_eq!(r.status.code(), Some(2));
    let r = run(&["seal", "--to", &p, "--label", "Bad Label", "-"], b"x");
    assert_eq!(r.status.code(), Some(1));
    let _ = std::fs::remove_dir_all(&d);
}
