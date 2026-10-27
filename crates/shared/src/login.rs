//! Login window <-> wol-login socket wire (IaraOS ADR 0024, WolOS ADR 0003).
//! Auth: `user\n` + password. Commands: `!verb\n` (+ body); `!` never starts an account name.
//! Reply: one line `ok|denied|error`.

pub const SOCKET: &str = "/var/run/wol-login.sock";
/// Password hints by account name (world-readable; the login window shows one after 3 failures).
pub const HINTS_DIR: &str = "/var/db/wolos/hints";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Power {
    Sleep,
    Restart,
    ShutDown,
}

impl Power {
    pub const ALL: [Power; 3] = [Power::Sleep, Power::Restart, Power::ShutDown];

    pub fn verb(self) -> &'static str {
        match self {
            Power::Sleep => "sleep",
            Power::Restart => "restart",
            Power::ShutDown => "shutdown",
        }
    }

    pub fn from_verb(v: &str) -> Option<Power> {
        Power::ALL.into_iter().find(|p| p.verb() == v)
    }
}

pub fn encode_power(p: Power) -> Vec<u8> {
    format!("!{}\n", p.verb()).into_bytes()
}

/// Splits `!verb\n<body>` into verb and body; None for auth requests.
pub fn command(data: &[u8]) -> Option<(&str, &[u8])> {
    let rest = data.strip_prefix(b"!")?;
    let (verb, body) = match rest.iter().position(|b| *b == b'\n') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, &[][..]),
    };
    Some((std::str::from_utf8(verb).ok()?, body))
}

/// One boot timeline line (ADR 0012), from wol-login and the first surfaces alike: uptime in ms
/// (CLOCK_MONOTONIC counts from boot on FreeBSD). `grep wolos-boot /var/log/messages`.
pub fn boot_mark(uptime_ms: u64, stage: &str) -> String {
    format!("wolos-boot: {uptime_ms} ms {stage}")
}

/// [`boot_mark`] for now.
pub fn boot_mark_now(stage: &str) -> String {
    boot_mark(crate::sys::monotonic_ns() / 1_000_000, stage)
}

/// ADR 0004 account request (6 fields); still accepted.
pub const SETUP_CREATE_USER: &str = "setup-create-user";
/// ADR 0013 account request: the 0004 fields, avatar, accent, then the password.
pub const SETUP_CREATE_ACCOUNT: &str = "setup-create-account";
pub const SETUP_WIFI: &str = "setup-wifi";
/// Avatar colour per account (one word), world-readable like [`HINTS_DIR`] (ADR 0013).
pub const AVATARS_DIR: &str = "/var/db/wolos/avatars";
/// Monogram colours the Setup Assistant offers; anything else is not written.
pub const AVATAR_TONES: [&str; 6] = ["violet", "blue", "green", "orange", "pink", "graphite"];
/// Theme accent when the request names none (`wol_core::theme::accent::DEFAULT`).
pub const DEFAULT_ACCENT: &str = "violet";

/// Setup assistant account (ADR 0004, 0011). Body: one field per line in this order, password last (rest).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct NewAccount {
    pub name: String,
    pub full_name: String,
    pub hint: String,
    /// e.g. `en_US.UTF-8`
    pub locale: String,
    /// xkb layout, e.g. `us`, `br`
    pub keymap: String,
    /// `light|dark|auto`
    pub appearance: String,
    /// Monogram colour shown on the login window and lock screen; empty = default.
    pub avatar: String,
    /// Theme accent id (`wol_core::theme::accent::ACCENTS`).
    pub accent: String,
    pub password: String,
}

impl NewAccount {
    pub fn encode(&self) -> Vec<u8> {
        let f = [
            &self.name,
            &self.full_name,
            &self.hint,
            &self.locale,
            &self.keymap,
            &self.appearance,
            &self.avatar,
            &self.accent,
        ];
        let mut b = format!("!{SETUP_CREATE_ACCOUNT}\n").into_bytes();
        for x in f {
            b.extend_from_slice(x.replace('\n', " ").as_bytes());
            b.push(b'\n');
        }
        b.extend_from_slice(self.password.as_bytes());
        b
    }

    /// Body after the verb line ([`SETUP_CREATE_ACCOUNT`] or the 0004 [`SETUP_CREATE_USER`]).
    /// None for another verb, missing fields or non-UTF-8.
    pub fn decode_verb(verb: &str, body: &[u8]) -> Option<NewAccount> {
        let v2 = match verb {
            SETUP_CREATE_ACCOUNT => true,
            SETUP_CREATE_USER => false,
            _ => return None,
        };
        let s = std::str::from_utf8(body).ok()?;
        let mut it = s.splitn(if v2 { 9 } else { 7 }, '\n');
        let mut f = || it.next().map(String::from);
        let (name, full_name, hint, locale, keymap, appearance) =
            (f()?, f()?, f()?, f()?, f()?, f()?);
        let (avatar, accent) = if v2 {
            (f()?, f()?)
        } else {
            (String::new(), DEFAULT_ACCENT.into())
        };
        Some(NewAccount {
            name,
            full_name,
            hint,
            locale,
            keymap,
            appearance,
            avatar,
            accent,
            password: f()?,
        })
    }
}

/// `!setup-wifi\nssid\npassword` (password may be empty: open network).
pub fn encode_wifi(ssid: &str, password: &str) -> Vec<u8> {
    format!("!{SETUP_WIFI}\n{ssid}\n{password}").into_bytes()
}

pub fn decode_wifi(body: &[u8]) -> Option<(String, Option<String>)> {
    let s = std::str::from_utf8(body).ok()?;
    let (ssid, pw) = s.split_once('\n')?;
    Some((ssid.into(), (!pw.is_empty()).then(|| pw.into())))
}

/// Setup's Wi-Fi page asks wol-login to scan (`_wollogin` cannot reach wpa_supplicant, ADR 0013).
pub const SETUP_WIFI_SCAN: &str = "setup-wifi-scan";

/// One network in the `!setup-wifi-scan` reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenNetwork {
    pub ssid: String,
    /// 0..=100.
    pub signal: u8,
    pub secure: bool,
    pub connected: bool,
}

pub fn encode_wifi_scan() -> Vec<u8> {
    format!("!{SETUP_WIFI_SCAN}\n").into_bytes()
}

/// Reply: `ok`, then one `signal\tsecure\tconnected\tssid` line per network (flags 0/1).
pub fn encode_networks(nets: &[SeenNetwork]) -> String {
    let mut s = String::from("ok");
    for n in nets.iter().filter(|n| !n.ssid.contains(['\n', '\r'])) {
        s.push_str(&format!(
            "\n{}\t{}\t{}\t{}",
            n.signal.min(100),
            u8::from(n.secure),
            u8::from(n.connected),
            n.ssid
        ));
    }
    s
}

/// Networks from a scan reply; None unless it starts with `ok`. Malformed lines are skipped.
pub fn decode_networks(reply: &str) -> Option<Vec<SeenNetwork>> {
    let mut lines = reply.lines();
    (lines.next()? == "ok").then(|| {
        lines
            .filter_map(|l| {
                let mut f = l.splitn(4, '\t');
                let (sig, sec, con, ssid) = (f.next()?, f.next()?, f.next()?, f.next()?);
                Some(SeenNetwork {
                    ssid: ssid.into(),
                    signal: sig.parse::<u8>().ok()?.min(100),
                    secure: sec == "1",
                    connected: con == "1",
                })
            })
            .filter(|n| !n.ssid.is_empty())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_marks_share_one_format() {
        assert_eq!(
            boot_mark(812, "lock shown"),
            "wolos-boot: 812 ms lock shown"
        );
        let now = boot_mark_now("x");
        let ms = now
            .strip_prefix("wolos-boot: ")
            .and_then(|r| r.strip_suffix(" ms x"));
        assert!(ms.is_some_and(|m| m.parse::<u64>().is_ok()), "{now}");
    }

    #[test]
    fn wifi_scan_round_trip() {
        assert_eq!(encode_wifi_scan(), b"!setup-wifi-scan\n");
        let nets = vec![
            SeenNetwork {
                ssid: "Rabbit Hole\t5G".into(),
                signal: 88,
                secure: true,
                connected: false,
            },
            SeenNetwork {
                ssid: "Tea Party".into(),
                signal: 30,
                secure: false,
                connected: true,
            },
        ];
        let wire = encode_networks(&nets);
        assert!(wire.starts_with("ok\n88\t1\t0\tRabbit Hole\t5G\n"));
        assert_eq!(decode_networks(&format!("{wire}\n")), Some(nets));
        assert_eq!(decode_networks("ok"), Some(vec![]));
        assert_eq!(decode_networks("error\n"), None);
        assert_eq!(
            decode_networks("ok\nloud\t1\t0\tX\n40\t0\t0\t\n").unwrap(),
            vec![]
        );
    }

    #[test]
    fn setup_round_trip() {
        let a = NewAccount {
            name: "alice".into(),
            full_name: "Alice\nLiddell".into(),
            hint: "rabbit".into(),
            locale: "en_GB.UTF-8".into(),
            keymap: "gb".into(),
            appearance: "auto".into(),
            avatar: String::new(),
            accent: DEFAULT_ACCENT.into(),
            password: "multi\nline".into(),
        };
        let e = a.encode();
        let (v, body) = command(&e).unwrap();
        assert_eq!(v, SETUP_CREATE_ACCOUNT);
        let d = NewAccount::decode_verb(v, body).unwrap();
        assert_eq!(d.full_name, "Alice Liddell");
        assert_eq!(d.password, "multi\nline");
        assert_eq!(d.keymap, "gb");
        assert_eq!(NewAccount::decode_verb(v, b"a\nb\nc"), None);
        let w = encode_wifi("Home", "secret12");
        let (v, body) = command(&w).unwrap();
        assert_eq!(v, SETUP_WIFI);
        assert_eq!(
            decode_wifi(body),
            Some(("Home".into(), Some("secret12".into())))
        );
        assert_eq!(decode_wifi(b"Cafe\n"), Some(("Cafe".into(), None)));
    }

    /// ADR 0013: `!setup-create-account` adds the avatar colour and the theme accent before the
    /// password. The 0004 verb still decodes (no avatar, default accent).
    #[test]
    fn setup_account_v2_and_the_0004_verb() {
        let a = NewAccount {
            name: "alice".into(),
            full_name: "Alice Liddell".into(),
            hint: "".into(),
            locale: "pt_BR.UTF-8".into(),
            keymap: "br".into(),
            appearance: "dark".into(),
            avatar: "green".into(),
            accent: "blue".into(),
            password: "curiouser\nand curiouser".into(),
        };
        let e = a.encode();
        let (v, body) = command(&e).unwrap();
        assert_eq!(v, SETUP_CREATE_ACCOUNT);
        assert_eq!(NewAccount::decode_verb(v, body), Some(a.clone()));
        let old = b"alice\nAlice\n\nen_US.UTF-8\nus\nauto\npw\nwith newline";
        let d = NewAccount::decode_verb(SETUP_CREATE_USER, old).unwrap();
        assert_eq!(
            (d.avatar.as_str(), d.accent.as_str(), d.password.as_str()),
            ("", DEFAULT_ACCENT, "pw\nwith newline")
        );
        assert_eq!(NewAccount::decode_verb("other", old), None);
        assert_eq!(NewAccount::decode_verb(SETUP_CREATE_ACCOUNT, b"a\nb"), None);
    }

    #[test]
    fn power_round_trip() {
        for p in Power::ALL {
            let e = encode_power(p);
            let (v, body) = command(&e).unwrap();
            assert_eq!((Power::from_verb(v), body), (Some(p), &b""[..]));
        }
        assert_eq!(command(b"ana\npw"), None);
        assert_eq!(command(b"!x\nbody"), Some(("x", &b"body"[..])));
    }
}
