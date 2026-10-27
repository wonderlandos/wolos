//! Start-up lock (ADR 0012) opens the login keyring with the password typed at the lock screen.
//! Real gnome-keyring on a private D-Bus; skipped where gnome-keyring-daemon or dbus-daemon is missing.
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const ROOT: &str = "/org/freedesktop/secrets";

fn have(bin: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {bin}")])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Private session bus + one gnome-keyring-daemon at a time, as the session runs it.
struct Session {
    bus: Child,
    keyring: Option<Child>,
    address: String,
    home: PathBuf,
    run: PathBuf,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop_keyring();
        let _ = self.bus.kill();
        let _ = self.bus.wait();
    }
}

impl Session {
    fn new(tag: &str) -> Session {
        // Short paths: the keyring control socket must fit sun_path.
        let root = std::env::temp_dir().join(format!("wkr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, run) = (root.join("h"), root.join("r"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&run).unwrap();
        let mut bus = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(bus.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Session {
            bus,
            keyring: None,
            address: address.trim().to_string(),
            home,
            run,
        }
    }

    fn connect(&self) -> Connection {
        zbus::blocking::connection::Builder::address(self.address.as_str())
            .unwrap()
            .build()
            .unwrap()
    }

    /// Fresh daemon (a boot): keyrings on disk load locked.
    fn start_keyring(&mut self) -> Connection {
        self.stop_keyring();
        let k = Command::new("gnome-keyring-daemon")
            .args(["--foreground", "--components=secrets"])
            .arg(format!("--control-directory={}", self.run.display()))
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/local/bin")
            .env("HOME", &self.home)
            .env("XDG_RUNTIME_DIR", &self.run)
            .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.keyring = Some(k);
        let conn = self.connect();
        let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        while !dbus
            .name_has_owner(shell::login::SECRETS.try_into().unwrap())
            .unwrap_or(false)
        {
            assert!(
                Instant::now() < end,
                "gnome-keyring did not take the bus name"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        conn
    }

    fn stop_keyring(&mut self) {
        if let Some(mut k) = self.keyring.take() {
            let _ = k.kill();
            let _ = k.wait();
        }
    }
}

fn plain_session(conn: &Connection) -> OwnedObjectPath {
    let r = conn
        .call_method(
            Some(shell::login::SECRETS),
            ROOT,
            Some("org.freedesktop.Secret.Service"),
            "OpenSession",
            &("plain", Value::from("")),
        )
        .unwrap();
    r.body()
        .deserialize::<(OwnedValue, OwnedObjectPath)>()
        .unwrap()
        .1
}

/// What wol-login's first login leaves behind: a login keyring with the account password.
fn create_login_keyring(conn: &Connection, password: &str) -> OwnedObjectPath {
    let s = plain_session(conn);
    let mut attrs: HashMap<&str, Value> = HashMap::new();
    // gnome-keyring --login names it `login` (login.keyring).
    attrs.insert(
        "org.freedesktop.Secret.Collection.Label",
        Value::from("login"),
    );
    let secret = (
        s.as_ref(),
        Vec::<u8>::new(),
        password.as_bytes(),
        "text/plain",
    );
    let r = conn
        .call_method(
            Some(shell::login::SECRETS),
            ROOT,
            Some("org.gnome.keyring.InternalUnsupportedGuiltRiddenInterface"),
            "CreateWithMasterPassword",
            &(attrs, secret),
        )
        .unwrap();
    r.body().deserialize::<OwnedObjectPath>().unwrap()
}

fn login_locked(conn: &Connection) -> bool {
    let r = conn
        .call_method(
            Some(shell::login::SECRETS),
            ObjectPath::try_from(shell::login::LOGIN_COLLECTION).unwrap(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.Secret.Collection", "Locked"),
        )
        .unwrap();
    bool::try_from(r.body().deserialize::<OwnedValue>().unwrap()).unwrap()
}

#[test]
fn startup_lock_opens_the_login_keyring() {
    if !have("gnome-keyring-daemon") || !have("dbus-daemon") {
        eprintln!("skipped: needs gnome-keyring-daemon and dbus-daemon");
        return;
    }
    let mut s = Session::new("unlock");
    let conn = s.start_keyring();
    let path = create_login_keyring(&conn, "s3cret");
    assert_eq!(path.as_str(), shell::login::LOGIN_COLLECTION);
    drop(conn);
    // Locked start: the session's daemon comes up with no password; the keyring is locked.
    let conn = s.start_keyring();
    assert!(
        login_locked(&conn),
        "a locked start leaves the login keyring locked"
    );
    assert!(shell::login::unlock_login_keyring(&conn, "wrong").is_err());
    assert!(login_locked(&conn));
    shell::login::unlock_login_keyring(&conn, "s3cret").unwrap();
    assert!(!login_locked(&conn), "the lock screen password opens it");
}
