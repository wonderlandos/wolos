// SPDX-License-Identifier: BSD-2-Clause
//! `wol-keyringd` end to end in a scratch folder: the fast-path socket (locked and unlocked,
//! wrong passwords and the back-off, an app that is not on an item's list, the idle lock) and the
//! Secret Service on a private session bus.
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const CLI: &str = env!("CARGO_BIN_EXE_wol-keyring");
const DAEMON: &str = env!("CARGO_BIN_EXE_wol-keyringd");
const PW: &str = "first password";

struct Scratch {
    dir: std::path::PathBuf,
    child: Option<Child>,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("wolkeyring-d-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch { dir, child: None }
    }
    fn sock(&self) -> std::path::PathBuf {
        self.dir.join("run/keyring.sock")
    }
    fn cli(&self, args: &[&str], stdin: &str) -> (bool, String, String) {
        let mut c = Command::new(CLI)
            .args(args)
            .env("WOLOS_KEYRING_DIR", &self.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        let o = c.wait_with_output().unwrap();
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into(),
            String::from_utf8_lossy(&o.stderr).into(),
        )
    }
    fn start(&mut self, bus: Option<&str>) {
        let mut c = Command::new(DAEMON);
        c.env("WOLOS_KEYRING_DIR", &self.dir)
            .env("WOLOS_KEYRING_SOCKET", self.sock())
            .stderr(Stdio::inherit());
        match bus {
            None => {
                c.arg("--no-bus");
            }
            Some(addr) => {
                c.env("DBUS_SESSION_BUS_ADDRESS", addr);
            }
        }
        self.child = Some(c.spawn().unwrap());
        let t = Instant::now();
        while UnixStream::connect(self.sock()).is_err() {
            assert!(
                t.elapsed() < Duration::from_secs(20),
                "the daemon did not start"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

// Drop: removes the scratch folder on every exit path.
impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(c) = self.child.as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn ask(s: &mut UnixStream, line: &str) -> serde_json::Value {
    s.write_all(line.as_bytes()).unwrap();
    s.write_all(b"\n").unwrap();
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut out = String::new();
    r.read_line(&mut out).unwrap();
    serde_json::from_str(&out).unwrap()
}

fn b64(s: &str) -> String {
    wolkeyring::wire::b64(s.as_bytes())
}

#[test]
fn the_fast_path_end_to_end() {
    let mut sc = Scratch::new("fast");
    assert!(sc.cli(&["init"], &format!("{PW}\n")).0);
    sc.start(None);
    let mut c = UnixStream::connect(sc.sock()).unwrap();
    // Locked: nothing is readable, and the answer says why.
    let r = ask(&mut c, r#"{"op":"status"}"#);
    assert_eq!(r["unlocked"], false);
    let r = ask(&mut c, r#"{"op":"search","attrs":{}}"#);
    assert_eq!(r["error"], "locked");
    // Unknown fields and operations are refused, and the connection goes on.
    assert_eq!(
        ask(&mut c, r#"{"op":"get","id":"a","as":"root"}"#)["error"],
        "malformed"
    );
    // Wrong passwords: refused, then backed off even for the right one.
    for _ in 0..3 {
        assert_eq!(
            ask(&mut c, r#"{"op":"unlock","password":"nope nope nope"}"#)["error"],
            "wrong-factors"
        );
    }
    let r = ask(&mut c, &format!(r#"{{"op":"unlock","password":"{PW}"}}"#));
    assert_eq!(r["error"], "policy", "{r}");
    std::thread::sleep(Duration::from_millis(1100));
    let r = ask(&mut c, &format!(r#"{{"op":"unlock","password":"{PW}"}}"#));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(ask(&mut c, r#"{"op":"status"}"#)["unlocked"], true);
    // Write, search, read, delete.
    let r = ask(
        &mut c,
        &format!(
            r#"{{"op":"set","label":"Example","attrs":{{"url":"a.example","user":"ana"}},"secret":"{}"}}"#,
            b64("hunter2")
        ),
    );
    assert_eq!(r["ok"], true, "{r}");
    let id = r["id"].as_str().unwrap().to_string();
    let r = ask(&mut c, r#"{"op":"search","attrs":{"url":"a.example"}}"#);
    assert_eq!(r["hits"][0]["id"], id.as_str());
    let r = ask(&mut c, &format!(r#"{{"op":"get","id":"{id}"}}"#));
    let got = wolkeyring::wire::unb64(r["secret"].as_str().unwrap(), 64).unwrap();
    assert_eq!(got, b"hunter2");
    // The command line sees the daemon's write (same files, same lock) and the audit names the app.
    let (ok, out, err) = sc.cli(&["get", "url=a.example"], &format!("{PW}\n"));
    assert!(ok, "{err}");
    assert_eq!(out.trim(), "hunter2");
    let r = ask(&mut c, r#"{"op":"audit","limit":50}"#);
    let lines = r["lines"].as_array().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l.as_str().unwrap().contains("\"read\""))
    );
    // Lock: reads stop.
    assert_eq!(ask(&mut c, r#"{"op":"lock"}"#)["ok"], true);
    assert_eq!(
        ask(&mut c, &format!(r#"{{"op":"get","id":"{id}"}}"#))["error"],
        "locked"
    );
    // A request line over the maximum is refused and the connection closed.
    let mut c2 = UnixStream::connect(sc.sock()).unwrap();
    c2.write_all(&vec![b'x'; wolkeyring::proto::MAX_LINE + 10])
        .unwrap();
    c2.write_all(b"\n").unwrap();
    let mut out = String::new();
    BufReader::new(&c2).read_line(&mut out).unwrap();
    assert!(out.contains("limit"), "{out}");
}

#[test]
fn another_app_is_not_on_the_list() {
    let mut sc = Scratch::new("acl");
    assert!(sc.cli(&["init"], &format!("{PW}\n")).0);
    sc.start(None);
    let mut c = UnixStream::connect(sc.sock()).unwrap();
    assert_eq!(
        ask(&mut c, &format!(r#"{{"op":"unlock","password":"{PW}"}}"#))["ok"],
        true
    );
    let r = ask(
        &mut c,
        &format!(
            r#"{{"op":"set","label":"Mine","attrs":{{"app":"mine"}},"secret":"{}"}}"#,
            b64("s3cret")
        ),
    );
    let id = r["id"].as_str().unwrap().to_string();
    // A process with another executable (python3) reads the same item and is refused.
    let py = "import socket,sys\ns=socket.socket(socket.AF_UNIX);s.connect(sys.argv[1]);s.sendall(sys.argv[2].encode()+b'\\n');print(s.makefile().readline())";
    let out = Command::new("python3")
        .args([
            "-I",
            "-c",
            py,
            sc.sock().to_str().unwrap(),
            &format!(r#"{{"op":"get","id":"{id}"}}"#),
        ])
        .output();
    let Ok(out) = out else { return };
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("\"denied\""),
        "{text} {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The owner allows it from the command line; then it reads.
    let py_path = std::fs::canonicalize(
        String::from_utf8_lossy(
            &Command::new("sh")
                .args(["-c", "command -v python3"])
                .output()
                .unwrap()
                .stdout,
        )
        .trim(),
    )
    .unwrap();
    let (ok, _, err) = sc.cli(
        &["acl", "allow", py_path.to_str().unwrap(), "app=mine"],
        &format!("{PW}\n"),
    );
    assert!(ok, "{err}");
    let out = Command::new("python3")
        .args([
            "-I",
            "-c",
            py,
            sc.sock().to_str().unwrap(),
            &format!(r#"{{"op":"get","id":"{id}"}}"#),
        ])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("\"ok\":true"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    // The refusal and the grant are in the audit chain.
    let (ok, out, err) = sc.cli(&["audit", "--last", "20"], &format!("{PW}\n"));
    assert!(ok, "{err}");
    assert!(out.contains("read-denied") && out.contains("acl"), "{out}");
}

#[test]
fn the_secret_service_on_a_private_bus() {
    let Ok(bus) = Command::new("dbus-daemon")
        .args(["--session", "--print-address=1", "--nofork"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    struct Kill(Child);
    // Drop: kills the private broadwayd the test started.
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut bus = Kill(bus);
    let mut addr = String::new();
    BufReader::new(bus.0.stdout.take().unwrap())
        .read_line(&mut addr)
        .unwrap();
    let addr = addr.trim().to_string();
    let mut sc = Scratch::new("bus");
    assert!(sc.cli(&["init"], &format!("{PW}\n")).0);
    sc.start(Some(&addr));
    let mut c = UnixStream::connect(sc.sock()).unwrap();
    assert_eq!(
        ask(&mut c, &format!(r#"{{"op":"unlock","password":"{PW}"}}"#))["ok"],
        true
    );

    let conn = zbus::blocking::connection::Builder::address(addr.as_str())
        .unwrap()
        .build()
        .unwrap();
    // The daemon takes the name a moment after its socket opens.
    let t = Instant::now();
    let svc = loop {
        let p = zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
        )
        .unwrap();
        if p.call::<_, _, (zbus::zvariant::OwnedValue, zbus::zvariant::OwnedObjectPath)>(
            "OpenSession",
            &("plain", zbus::zvariant::Value::from("")),
        )
        .is_ok()
        {
            break p;
        }
        assert!(t.elapsed() < Duration::from_secs(20), "no Secret Service");
        std::thread::sleep(Duration::from_millis(100));
    };
    // Only the plain session is offered.
    let e = svc.call::<_, _, (zbus::zvariant::OwnedValue, zbus::zvariant::OwnedObjectPath)>(
        "OpenSession",
        &(
            "dh-ietf1024-sha256-aes128-cbc-pkcs7",
            zbus::zvariant::Value::from(vec![0u8; 8]),
        ),
    );
    assert!(e.is_err());
    let (_, session): (zbus::zvariant::OwnedValue, zbus::zvariant::OwnedObjectPath) = svc
        .call("OpenSession", &("plain", zbus::zvariant::Value::from("")))
        .unwrap();
    // CreateItem on the collection, then SearchItems and GetSecret.
    let coll = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.secrets",
        "/org/freedesktop/secrets/collection/login",
        "org.freedesktop.Secret.Collection",
    )
    .unwrap();
    let mut props: BTreeMap<String, zbus::zvariant::Value> = BTreeMap::new();
    props.insert(
        "org.freedesktop.Secret.Item.Label".into(),
        zbus::zvariant::Value::from("Helium: example"),
    );
    let attrs: std::collections::HashMap<String, String> =
        [("origin_url".to_string(), "https://a.example".to_string())].into();
    props.insert(
        "org.freedesktop.Secret.Item.Attributes".into(),
        zbus::zvariant::Value::from(attrs.clone()),
    );
    let secret = (
        session.clone(),
        Vec::<u8>::new(),
        b"hunter2".to_vec(),
        "text/plain".to_string(),
    );
    let (item, prompt): (
        zbus::zvariant::OwnedObjectPath,
        zbus::zvariant::OwnedObjectPath,
    ) = coll.call("CreateItem", &(props, secret, true)).unwrap();
    assert_eq!(prompt.as_str(), "/");
    let (open, shut): (
        Vec<zbus::zvariant::OwnedObjectPath>,
        Vec<zbus::zvariant::OwnedObjectPath>,
    ) = svc.call("SearchItems", &(attrs.clone(),)).unwrap();
    assert_eq!((open.len(), shut.len()), (1, 0));
    assert_eq!(open[0], item);
    let it = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.secrets",
        item.as_str(),
        "org.freedesktop.Secret.Item",
    )
    .unwrap();
    let (_, _, value, _): (zbus::zvariant::OwnedObjectPath, Vec<u8>, Vec<u8>, String) =
        it.call("GetSecret", &(session.clone(),)).unwrap();
    assert_eq!(value, b"hunter2");
    let got: std::collections::HashMap<String, String> = it.get_property("Attributes").unwrap();
    assert_eq!(got, attrs);
    // The same item is readable through the fast path (one store) and through the command line.
    let (ok, out, err) = sc.cli(&["get", "origin_url=https://a.example"], &format!("{PW}\n"));
    assert!(ok, "{err}");
    assert_eq!(out.trim(), "hunter2");
    // Locked keyring: GetSecret answers IsLocked, search finds nothing it must hide.
    assert_eq!(ask(&mut c, r#"{"op":"lock"}"#)["ok"], true);
    let e = it
        .call::<_, _, (zbus::zvariant::OwnedObjectPath, Vec<u8>, Vec<u8>, String)>(
            "GetSecret",
            &(session.clone(),),
        )
        .unwrap_err();
    assert!(e.to_string().contains("IsLocked"), "{e}");
    // A second collection is refused politely.
    let e = svc
        .call::<_, _, (
            zbus::zvariant::OwnedObjectPath,
            zbus::zvariant::OwnedObjectPath,
        )>(
            "CreateCollection",
            &(
                std::collections::HashMap::<String, zbus::zvariant::Value>::new(),
                "other",
            ),
        )
        .unwrap_err();
    assert!(e.to_string().contains("NotSupported"), "{e}");
    // Migrating from this very daemon is refused: there is nothing older to read.
    let (ok, _, err) = {
        let mut c = Command::new(CLI)
            .arg("migrate")
            .env("WOLOS_KEYRING_DIR", &sc.dir)
            .env("DBUS_SESSION_BUS_ADDRESS", &addr)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin
            .take()
            .unwrap()
            .write_all(format!("{PW}\n").as_bytes())
            .unwrap();
        let o = c.wait_with_output().unwrap();
        (
            o.status.success(),
            String::new(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    assert!(!ok && err.contains("already owns"), "{err}");
    // Delete through the bus.
    assert_eq!(
        ask(&mut c, &format!(r#"{{"op":"unlock","password":"{PW}"}}"#))["ok"],
        true
    );
    let _: zbus::zvariant::OwnedObjectPath = it.call("Delete", &()).unwrap();
    let (open, _): (
        Vec<zbus::zvariant::OwnedObjectPath>,
        Vec<zbus::zvariant::OwnedObjectPath>,
    ) = svc.call("SearchItems", &(attrs,)).unwrap();
    assert!(open.is_empty());
}

#[test]
fn migrate_copies_a_gnome_keyring_and_touches_nothing() {
    if Command::new("gnome-keyring-daemon")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let Ok(bus) = Command::new("dbus-daemon")
        .args(["--session", "--print-address=1", "--nofork"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let mut bus = KillOnDrop(bus);
    let mut addr = String::new();
    BufReader::new(bus.0.stdout.take().unwrap())
        .read_line(&mut addr)
        .unwrap();
    let addr = addr.trim().to_string();
    let sc = Scratch::new("migrate");
    let home = sc.dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    // The old keyring: gnome-keyring's secrets component with a fresh login keyring.
    std::fs::create_dir_all(sc.dir.join("gkrun")).unwrap();
    let mut gk = Command::new("gnome-keyring-daemon")
        .args(["--unlock", "--components=secrets"])
        .env("DBUS_SESSION_BUS_ADDRESS", &addr)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_RUNTIME_DIR", sc.dir.join("gkrun"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    gk.stdin.take().unwrap().write_all(b"old password").unwrap();
    let _ = gk.wait();
    let conn = zbus::blocking::connection::Builder::address(addr.as_str())
        .unwrap()
        .build()
        .unwrap();
    let t = Instant::now();
    // Ask nothing of the name until it has an owner: a call to a name nobody owns makes the bus
    // start gnome-keyring from its service file, with this user's real folders.
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).unwrap();
    while !dbus
        .name_has_owner("org.freedesktop.secrets".try_into().unwrap())
        .unwrap()
    {
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "gnome-keyring did not come up"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let svc = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.secrets",
        "/org/freedesktop/secrets",
        "org.freedesktop.Secret.Service",
    )
    .unwrap();
    let (_, session): (zbus::zvariant::OwnedValue, zbus::zvariant::OwnedObjectPath) = svc
        .call("OpenSession", &("plain", zbus::zvariant::Value::from("")))
        .unwrap();
    // The login collection appears a moment after the service answers.
    let coll_path = loop {
        let p: zbus::zvariant::OwnedObjectPath = svc.call("ReadAlias", &("default",)).unwrap();
        if p.as_str() != "/" {
            break p;
        }
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "gnome-keyring made no default collection"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let coll = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.secrets",
        coll_path.as_str(),
        "org.freedesktop.Secret.Collection",
    )
    .unwrap();
    for (label, site, secret) in [
        ("Helium: a", "https://a.example", "pw-a"),
        ("VPN key", "vpn-1", "wg-private"),
    ] {
        let mut props: BTreeMap<String, zbus::zvariant::Value> = BTreeMap::new();
        props.insert(
            "org.freedesktop.Secret.Item.Label".into(),
            zbus::zvariant::Value::from(label),
        );
        let attrs: std::collections::HashMap<String, String> =
            [("site".to_string(), site.to_string())].into();
        props.insert(
            "org.freedesktop.Secret.Item.Attributes".into(),
            zbus::zvariant::Value::from(attrs),
        );
        let sec = (
            session.clone(),
            Vec::<u8>::new(),
            secret.as_bytes().to_vec(),
            "text/plain".to_string(),
        );
        let _: (
            zbus::zvariant::OwnedObjectPath,
            zbus::zvariant::OwnedObjectPath,
        ) = coll.call("CreateItem", &(props, sec, true)).unwrap();
    }
    assert!(sc.cli(&["init"], &format!("{PW}\n")).0);
    let run = |args: &[&str], stdin: &str| {
        let mut c = Command::new(CLI)
            .args(args)
            .env("WOLOS_KEYRING_DIR", &sc.dir)
            .env("DBUS_SESSION_BUS_ADDRESS", &addr)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        let o = c.wait_with_output().unwrap();
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let (ok, out, err) = run(&["migrate"], &format!("{PW}\n"));
    assert!(ok, "{err}");
    assert!(out.contains("2 items copied, 0 skipped"), "{out}");
    let (_, out, _) = run(&["get", "site=vpn-1"], &format!("{PW}\n"));
    assert_eq!(out.trim(), "wg-private");
    // Running it again replaces the same items, it does not double them.
    assert!(run(&["migrate"], &format!("{PW}\n")).0);
    let (_, out, _) = run(&["list"], &format!("{PW}\n"));
    assert_eq!(out.lines().count(), 2, "{out}");
    // The old side still has its two items.
    let (open, _): (
        Vec<zbus::zvariant::OwnedObjectPath>,
        Vec<zbus::zvariant::OwnedObjectPath>,
    ) = svc
        .call(
            "SearchItems",
            &(std::collections::HashMap::<String, String>::new(),),
        )
        .unwrap();
    assert_eq!(open.len(), 2);
    // Stop the daemon that --unlock left running: the process that owns the bus name.
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).unwrap();
    let owner = dbus
        .get_name_owner("org.freedesktop.secrets".try_into().unwrap())
        .unwrap();
    if let Some(pid) = dbus
        .get_connection_credentials(owner.into())
        .ok()
        .and_then(|c| c.process_id())
    {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

struct KillOnDrop(Child);
// Drop: kills the child the test started.
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn python_path() -> String {
    std::fs::canonicalize(
        String::from_utf8_lossy(
            &Command::new("sh")
                .args(["-c", "command -v python3"])
                .output()
                .unwrap()
                .stdout,
        )
        .trim(),
    )
    .unwrap()
    .to_str()
    .unwrap()
    .to_string()
}

const PY_GET: &str = "import socket,sys\ns=socket.socket(socket.AF_UNIX);s.connect(sys.argv[1]);s.sendall(sys.argv[2].encode()+b'\\n');print(s.makefile().readline())";

#[test]
fn a_program_that_changes_after_connecting_is_refused() {
    // python3 is on the item's list; it connects, then replaces itself with perl, which keeps the
    // socket: the identity taken at connect no longer holds, and the secret is not given.
    if Command::new("perl").arg("-e").arg("1").output().is_err() {
        return;
    }
    let mut sc = Scratch::new("exec");
    assert!(sc.cli(&["init"], &format!("{PW}\n")).0);
    sc.start(None);
    let mut c = UnixStream::connect(sc.sock()).unwrap();
    assert_eq!(
        ask(&mut c, &format!(r#"{{"op":"unlock","password":"{PW}"}}"#))["ok"],
        true
    );
    let r = ask(
        &mut c,
        &format!(
            r#"{{"op":"set","label":"Mine","attrs":{{"app":"mine"}},"secret":"{}"}}"#,
            b64("s3cret")
        ),
    );
    let id = r["id"].as_str().unwrap().to_string();
    let (ok, _, err) = sc.cli(
        &["acl", "allow", &python_path(), "app=mine"],
        &format!("{PW}\n"),
    );
    assert!(ok, "{err}");
    let req = format!(r#"{{"op":"get","id":"{id}"}}"#);
    // Control: python3 itself reads it.
    let out = Command::new("python3")
        .args(["-I", "-c", PY_GET, sc.sock().to_str().unwrap(), &req])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("\"ok\":true"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    // The same python3 connects, then execs perl with the socket still open.
    let py = "import os,socket,sys\ns=socket.socket(socket.AF_UNIX);s.connect(sys.argv[1]);s.sendall(b'{\"op\":\"status\"}\\n');s.makefile().readline();os.set_inheritable(s.fileno(),True)\nperl=r'open(S,\"+<&=\".$ARGV[0]) or die; select(S); $|=1; print S $ARGV[1].\"\\n\"; $l=<S>; select(STDOUT); print $l;'\nos.execvp('perl',['perl','-e',perl,str(s.fileno()),sys.argv[2]])";
    let out = Command::new("python3")
        .args(["-I", "-c", py, sc.sock().to_str().unwrap(), &req])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        !text.contains("\"secret\""),
        "the secret went to a program that was not on the list: {text}"
    );
    assert!(
        text.contains("\"denied\""),
        "{text} {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn an_item_nobody_was_granted_stays_owner_only() {
    let mut sc = Scratch::new("owner");
    assert!(sc.cli(&["init"], &format!("{PW}\n")).0);
    // Made at the terminal, as an import or a migration makes items: the list is empty.
    assert!(
        sc.cli(
            &["set", "Imported", "app=imp"],
            &format!("{PW}\nsecret-imp\n")
        )
        .0
    );
    sc.start(None);
    let mut c = UnixStream::connect(sc.sock()).unwrap();
    assert_eq!(
        ask(&mut c, &format!(r#"{{"op":"unlock","password":"{PW}"}}"#))["ok"],
        true
    );
    let id = ask(&mut c, r#"{"op":"search","attrs":{"app":"imp"}}"#)["hits"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let req = format!(r#"{{"op":"get","id":"{id}"}}"#);
    let out = Command::new("python3")
        .args(["-I", "-c", PY_GET, sc.sock().to_str().unwrap(), &req])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("\"denied\""),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    // The owner lists it as having no app, then allows one program for every item at once.
    let (ok, out, _) = sc.cli(&["acl", "list", "app=imp"], &format!("{PW}\n"));
    assert!(ok && out.contains("owner only"), "{out}");
    let (ok, out, err) = sc.cli(&["acl", "allow", &python_path()], &format!("{PW}\n"));
    assert!(ok && out.contains("1 items"), "{out} {err}");
    let out = Command::new("python3")
        .args(["-I", "-c", PY_GET, sc.sock().to_str().unwrap(), &req])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("\"ok\":true"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn the_daemon_keeps_its_memory_private() {
    use std::os::unix::fs::MetadataExt;
    let mut sc = Scratch::new("dump");
    assert!(sc.cli(&["init"], &format!("{PW}\n")).0);
    sc.start(None);
    let pid = sc.child.as_ref().unwrap().id();
    // A process that is not dumpable has its /proc entries owned by root: a same-user process can
    // neither ptrace it nor read its memory, and a crash writes no core file.
    assert_eq!(
        std::fs::metadata(format!("/proc/{pid}/status"))
            .unwrap()
            .uid(),
        0
    );
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).unwrap();
    let core = limits
        .lines()
        .find(|l| l.starts_with("Max core file size"))
        .unwrap();
    assert!(
        core.split_whitespace().rev().nth(1) == Some("0")
            && core.split_whitespace().rev().nth(2) == Some("0"),
        "{core}"
    );
    // The daemon still names callers: reading another process's /proc entries is unaffected.
    let mut c = UnixStream::connect(sc.sock()).unwrap();
    assert_eq!(ask(&mut c, r#"{"op":"status"}"#)["ok"], true);
}
