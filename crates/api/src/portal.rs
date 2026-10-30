//! wold portal (ADR 0020): libwol service calls. The library is unprivileged and asks; wold decides.
//! Who asks comes from the kernel (peer uid, executable of the peer pid), never from the message.
//! Grants are the consent database the person controls in Settings > Privacy & Security.
use crate::bus::Bus;
use shared::events::Event;
use shared::portal::{Answer, Call, Code};
use std::io::Write;
use std::path::PathBuf;

/// The calling process as the kernel reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caller {
    pub uid: u32,
    /// Canonical executable path.
    pub app: String,
}

pub trait Grants {
    fn granted(&self, app: &str, capability: &str) -> bool;
    /// May receive the account's notifications (`subscribe account`).
    fn shows_notifications(&self, app: &str) -> bool;
}

/// Programs that show notifications: the shell.
pub const SHOWERS: [&str; 1] = ["/usr/bin/wol-shell"];

/// `wol_core::consent::FILE`, read on every call so a change in Settings applies at once.
pub struct ConsentFile(pub PathBuf);

impl Grants for ConsentFile {
    fn granted(&self, app: &str, capability: &str) -> bool {
        std::fs::read_to_string(&self.0)
            .map(|t| wol_core::consent::parse(&t))
            .unwrap_or_default()
            .iter()
            .any(|(a, c)| a == app && c == capability)
    }

    fn shows_notifications(&self, app: &str) -> bool {
        SHOWERS.contains(&app)
    }
}

/// Person-facing name of an app path (`/usr/bin/foo` -> `foo`).
fn name(app: &str) -> &str {
    app.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(app)
}

/// One call line's JSON -> answer. Nothing happens unless the call decodes, the caller is known
/// and the caller's executable holds the call's capability.
pub fn handle<W: Write + Send>(
    json: &str,
    caller: Option<&Caller>,
    grants: &dyn Grants,
    bus: &Bus<W>,
) -> Answer {
    let call = match Call::decode(json.as_bytes()) {
        Ok(c) => c,
        Err(m) => return Answer::err(Code::InvalidArgument, m),
    };
    let Some(caller) = caller else {
        return Answer::err(
            Code::PermissionDenied,
            "WolOS could not tell which app asked, so it did nothing.",
        );
    };
    if !grants.granted(&caller.app, call.capability()) {
        return Answer::err(
            Code::PermissionDenied,
            format!(
                "Notifications are off for {}. Turn them on in System Settings > Privacy & Security.",
                name(&caller.app)
            ),
        );
    }
    match call {
        Call::NotificationPost { title, body } => {
            let e = Event::Notification {
                uid: caller.uid,
                app: caller.app.clone(),
                title,
                body,
            };
            if bus.publish(&e) == 0 {
                return Answer::err(
                    Code::Unavailable,
                    "Notifications cannot be shown right now: the desktop is not running.",
                );
            }
            Answer::ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    struct Fixed(&'static [(&'static str, &'static str)]);
    impl Grants for Fixed {
        fn granted(&self, app: &str, cap: &str) -> bool {
            self.0.iter().any(|(a, c)| *a == app && *c == cap)
        }
        fn shows_notifications(&self, _: &str) -> bool {
            false
        }
    }

    const POST: &str = r#"{"call":"notification.post","title":"Done","body":"ok"}"#;

    fn caller(app: &str) -> Caller {
        Caller {
            uid: 1000,
            app: app.into(),
        }
    }

    #[test]
    fn decision_table() {
        let grants = Fixed(&[("/usr/bin/good", "notifications")]);
        let (good, bad) = (caller("/usr/bin/good"), caller("/usr/bin/other"));
        let cases: [(&str, Option<&Caller>, Option<Code>); 5] = [
            (POST, Some(&good), None),
            (POST, Some(&bad), Some(Code::PermissionDenied)),
            (POST, None, Some(Code::PermissionDenied)),
            (
                r#"{"call":"notification.post","title":""}"#,
                Some(&good),
                Some(Code::InvalidArgument),
            ),
            ("junk", Some(&good), Some(Code::InvalidArgument)),
        ];
        for (json, who, want) in cases {
            let bus = Bus::default();
            let (s, p) = UnixStream::pair().unwrap();
            bus.subscribe(Some(1000), s);
            let a = handle(json, who, &grants, &bus);
            assert_eq!(a.code, want, "{json} {who:?}");
            p.set_nonblocking(true).unwrap();
            let mut line = String::new();
            let delivered = BufReader::new(&p).read_line(&mut line).is_ok_and(|n| n > 0);
            assert_eq!(
                delivered,
                want.is_none(),
                "only an allowed call reaches the bus"
            );
        }
        let a = handle(POST, Some(&bad), &grants, &Bus::<UnixStream>::default());
        assert_eq!(
            a.message.as_deref(),
            Some(
                "Notifications are off for other. Turn them on in System Settings > Privacy & Security."
            )
        );
    }

    #[test]
    fn delivered_event_names_the_kernel_identity() {
        let bus = Bus::default();
        let (s, p) = UnixStream::pair().unwrap();
        bus.subscribe(Some(1000), s);
        let grants = Fixed(&[("/usr/bin/good", "notifications")]);
        assert!(handle(POST, Some(&caller("/usr/bin/good")), &grants, &bus).ok);
        let mut line = String::new();
        BufReader::new(p).read_line(&mut line).unwrap();
        assert_eq!(
            Event::parse(&line),
            Some(Event::Notification {
                uid: 1000,
                app: "/usr/bin/good".into(),
                title: "Done".into(),
                body: "ok".into(),
            })
        );
        let nobody = Bus::<UnixStream>::default();
        let a = handle(POST, Some(&caller("/usr/bin/good")), &grants, &nobody);
        assert_eq!(a.code, Some(Code::Unavailable));
    }

    #[test]
    fn consent_file_grants() {
        let dir = std::env::temp_dir().join(format!("wol-portal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("consent.json");
        let g = ConsentFile(f.clone());
        assert!(
            !g.granted("/usr/bin/a", "notifications"),
            "missing file grants nothing"
        );
        std::fs::write(
            &f,
            r#"{"grants":[{"app":"/usr/bin/a","capability":"notifications"}]}"#,
        )
        .unwrap();
        assert!(g.granted("/usr/bin/a", "notifications"));
        assert!(!g.granted("/usr/bin/a", "screen-capture"));
        std::fs::write(&f, "{broken").unwrap();
        assert!(!g.granted("/usr/bin/a", "notifications"));
        assert!(g.shows_notifications("/usr/bin/wol-shell"));
        assert!(!g.shows_notifications("/usr/bin/a"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
