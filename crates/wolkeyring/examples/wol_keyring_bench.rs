// SPDX-License-Identifier: BSD-2-Clause
//! `wol-keyring-bench`: get, set and search latency of a keyring (docs/bench/keyring.md).
//!
//!   wol-keyring-bench inproc [--items N] [--ops K]   the library, in this process
//!   wol-keyring-bench socket [--items N] [--ops K]   the fast path of an in-process daemon
//!   wol-keyring-bench bus    [--items N] [--ops K]   whoever owns org.freedesktop.secrets on the
//!                                                    session bus (wol-keyringd or gnome-keyring),
//!                                                    already unlocked
//!
//! `inproc` and `socket` use a scratch folder. Each prints one line per operation:
//! `mode | op | count | p50_us | p99_us`. Items are 64-byte secrets with three attributes.
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::time::Instant;

fn pct(v: &mut [u128], p: f64) -> u128 {
    v.sort_unstable();
    v[(((v.len() - 1) as f64) * p).round() as usize]
}

fn report(mode: &str, op: &str, mut t: Vec<u128>) {
    let n = t.len();
    let (a, b) = (pct(&mut t, 0.5), pct(&mut t, 0.99));
    println!(
        "{mode} | {op} | {n} | {:.1} | {:.1}",
        a as f64 / 1000.0,
        b as f64 / 1000.0
    );
}

/// A deterministic spread of indexes (no rng crate): a multiplicative walk modulo a prime.
fn pick(i: usize, n: usize) -> usize {
    (i.wrapping_mul(2_654_435_761).wrapping_add(12345)) % n
}

fn attrs(i: usize) -> BTreeMap<String, String> {
    [
        ("application".to_string(), "bench".to_string()),
        ("site".to_string(), format!("https://site{i}.example")),
        ("user".to_string(), format!("user{}", i % 97)),
    ]
    .into()
}

fn secret(i: usize) -> Vec<u8> {
    format!("{i:064}").into_bytes()
}

fn arg(a: &[String], name: &str, default: usize) -> usize {
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn inproc(items: usize, ops: usize) {
    use wolkeyring::item::Class;
    let (mut s, _) = wolkeyring::service::Session::create("bench", "bench password 1").unwrap();
    let mut ids = Vec::with_capacity(items);
    let mut t = Vec::new();
    for i in 0..items {
        let a = Instant::now();
        ids.push(
            s.set(
                None,
                &format!("item {i}"),
                attrs(i),
                &secret(i),
                Class::WhenUnlocked,
            )
            .unwrap(),
        );
        t.push(a.elapsed().as_nanos());
    }
    report("inproc", "set(new)", t);
    let mut t = Vec::new();
    for i in 0..ops {
        let k = pick(i, items);
        let a = Instant::now();
        let v = s.get(None, &ids[k]).unwrap();
        t.push(a.elapsed().as_nanos());
        assert_eq!(&v[..], &secret(k)[..]);
    }
    report("inproc", "get", t);
    let mut t = Vec::new();
    for i in 0..ops {
        let k = pick(i + 7, items);
        let q: BTreeMap<String, String> =
            [("site".to_string(), format!("https://site{k}.example"))].into();
        let a = Instant::now();
        let r = s.keyring.search(&q);
        t.push(a.elapsed().as_nanos());
        assert_eq!(r.len(), 1);
    }
    report("inproc", "search(1 attr, unique)", t);
    let mut t = Vec::new();
    for i in 0..ops {
        let k = pick(i + 11, items);
        let q: BTreeMap<String, String> = [
            ("application".to_string(), "bench".to_string()),
            ("user".to_string(), format!("user{}", k % 97)),
        ]
        .into();
        let a = Instant::now();
        let r = s.keyring.search(&q);
        t.push(a.elapsed().as_nanos());
        assert!(!r.is_empty());
    }
    report("inproc", "search(2 attrs, ~N/97 hits)", t);
    let mut t = Vec::new();
    for i in 0..ops {
        let k = pick(i + 3, items);
        let a = Instant::now();
        s.set(
            None,
            &format!("item {k}"),
            attrs(k),
            &secret(k + 1),
            Class::WhenUnlocked,
        )
        .unwrap();
        t.push(a.elapsed().as_nanos());
    }
    report("inproc", "set(replace)", t);
}

fn line(
    s: &mut std::os::unix::net::UnixStream,
    r: &mut BufReader<std::os::unix::net::UnixStream>,
    req: &str,
) -> serde_json::Value {
    s.write_all(req.as_bytes()).unwrap();
    s.write_all(b"\n").unwrap();
    let mut out = String::new();
    r.read_line(&mut out).unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    v
}

fn socket(items: usize, ops: usize) {
    let (mut sess, _) = wolkeyring::service::Session::create("bench", "bench password 1").unwrap();
    let _ = sess.lock("bench");
    drop(sess);
    let d = std::sync::Arc::new(wolkeyring::daemon::Daemon::new("bench"));
    let path = wolkeyring::io::socket_path().unwrap();
    let l = wolkeyring::daemon::bind(&path).unwrap();
    {
        let d = d.clone();
        std::thread::spawn(move || wolkeyring::daemon::serve(d, l));
    }
    let mut s = std::os::unix::net::UnixStream::connect(&path).unwrap();
    let mut r = BufReader::new(s.try_clone().unwrap());
    line(
        &mut s,
        &mut r,
        r#"{"op":"unlock","password":"bench password 1"}"#,
    );
    let mut ids = Vec::with_capacity(items);
    let mut t = Vec::new();
    for i in 0..items {
        let req = serde_json::json!({"op":"set","label":format!("item {i}"),"attrs":attrs(i),"secret":wolkeyring::wire::b64(&secret(i))}).to_string();
        let a = Instant::now();
        let v = line(&mut s, &mut r, &req);
        t.push(a.elapsed().as_nanos());
        ids.push(v["id"].as_str().unwrap().to_string());
    }
    report("socket", "set(new)", t);
    let mut t = Vec::new();
    for i in 0..ops {
        let k = pick(i, items);
        let req = format!(r#"{{"op":"get","id":"{}"}}"#, ids[k]);
        let a = Instant::now();
        let v = line(&mut s, &mut r, &req);
        t.push(a.elapsed().as_nanos());
        assert_eq!(
            wolkeyring::wire::unb64(v["secret"].as_str().unwrap(), 128).unwrap(),
            secret(k)
        );
    }
    report("socket", "get", t);
    let mut t = Vec::new();
    for i in 0..ops {
        let k = pick(i + 7, items);
        let req =
            serde_json::json!({"op":"search","attrs":{"site":format!("https://site{k}.example")}})
                .to_string();
        let a = Instant::now();
        let v = line(&mut s, &mut r, &req);
        t.push(a.elapsed().as_nanos());
        assert_eq!(v["hits"].as_array().unwrap().len(), 1);
    }
    report("socket", "search(1 attr, unique)", t);
}

fn bus(items: usize, ops: usize) {
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
    let conn = zbus::blocking::Connection::session().expect("session bus");
    let svc = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.secrets",
        "/org/freedesktop/secrets",
        "org.freedesktop.Secret.Service",
    )
    .unwrap();
    let (_, session): (OwnedValue, OwnedObjectPath) = svc
        .call("OpenSession", &("plain", Value::from("")))
        .unwrap();
    let coll_path: OwnedObjectPath = svc.call("ReadAlias", &("default",)).unwrap();
    let coll = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.secrets",
        coll_path.as_str(),
        "org.freedesktop.Secret.Collection",
    )
    .unwrap();
    let mut t = Vec::new();
    let mut paths: Vec<OwnedObjectPath> = Vec::new();
    for i in 0..items {
        let mut props: HashMap<String, Value> = HashMap::new();
        props.insert(
            "org.freedesktop.Secret.Item.Label".into(),
            Value::from(format!("item {i}")),
        );
        let at: HashMap<String, String> = attrs(i).into_iter().collect();
        props.insert(
            "org.freedesktop.Secret.Item.Attributes".into(),
            Value::from(at),
        );
        let sec = (
            session.clone(),
            Vec::<u8>::new(),
            secret(i),
            "text/plain".to_string(),
        );
        let a = Instant::now();
        let (p, _): (OwnedObjectPath, OwnedObjectPath) =
            coll.call("CreateItem", &(props, sec, true)).unwrap();
        t.push(a.elapsed().as_nanos());
        paths.push(p);
    }
    report("bus", "set(new)", t);
    let mut t = Vec::new();
    for i in 0..ops {
        let k = pick(i, items);
        let it = zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.secrets",
            paths[k].as_str(),
            "org.freedesktop.Secret.Item",
        )
        .unwrap();
        let a = Instant::now();
        let (_, _, v, _): (OwnedObjectPath, Vec<u8>, Vec<u8>, String) =
            it.call("GetSecret", &(session.clone(),)).unwrap();
        t.push(a.elapsed().as_nanos());
        assert_eq!(v, secret(k));
    }
    report("bus", "get", t);
    let mut t = Vec::new();
    for i in 0..ops {
        let k = pick(i + 7, items);
        let q: HashMap<String, String> =
            [("site".to_string(), format!("https://site{k}.example"))].into();
        let a = Instant::now();
        let (open, _): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            svc.call("SearchItems", &(q,)).unwrap();
        t.push(a.elapsed().as_nanos());
        assert_eq!(open.len(), 1);
    }
    report("bus", "search(1 attr, unique)", t);
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let items = arg(&a, "--items", 10_000);
    let ops = arg(&a, "--ops", 2_000);
    match a.first().map(String::as_str) {
        Some("inproc") | Some("socket") => {
            let dir = std::env::temp_dir().join(format!("wolkeyring-bench-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            with_scratch_env(&dir);
            if a[0] == "inproc" {
                inproc(items, ops)
            } else {
                socket(items, ops)
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
        Some("bus") => bus(items, ops),
        _ => {
            eprintln!("usage: wol-keyring-bench inproc|socket|bus [--items N] [--ops K]");
            std::process::exit(2);
        }
    }
}

/// `set_var` is unsafe in edition 2024; the library reads the folder from the environment only,
/// so the bench re-executes itself with the variables set instead of mutating its own.
fn with_scratch_env(dir: &std::path::Path) {
    if std::env::var_os("WOLOS_KEYRING_DIR").is_some() {
        return;
    }
    let st = std::process::Command::new(std::env::current_exe().unwrap())
        .args(std::env::args().skip(1))
        .env("WOLOS_KEYRING_DIR", dir)
        .env("WOLOS_KEYRING_SOCKET", dir.join("run/keyring.sock"))
        .status()
        .unwrap();
    let _ = std::fs::remove_dir_all(dir);
    std::process::exit(st.code().unwrap_or(1));
}
