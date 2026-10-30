//! wold portal (ADR 0020): libwol service calls. The library is unprivileged and asks; wold decides.
//! Who asks comes from the kernel (peer uid, executable of the peer pid), never from the message.
//! Grants are the consent database the person controls in Settings > Privacy & Security.
use crate::bus::Bus;
use shared::events::Event;
use shared::portal::{Answer, Call, Code, Need};
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// The calling process as the kernel reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caller {
    pub uid: u32,
    pub pid: i32,
    /// Canonical executable path.
    pub app: String,
}

/// Recent notifications: id -> (account, process that posted it), so a pressed button reaches
/// only that process. Bounded: the oldest are forgotten.
#[derive(Default)]
pub struct Notes {
    /// Last id given, and (id, uid, pid) newest last.
    inner: Mutex<(u32, VecDeque<Note>)>,
}

type Note = (u32, u32, i32);

pub const NOTES_KEPT: usize = 1024;

impl Notes {
    fn add(&self, uid: u32, pid: i32) -> u32 {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.0 = g.0.wrapping_add(1).max(1);
        let id = g.0;
        if g.1.len() == NOTES_KEPT {
            g.1.pop_front();
        }
        g.1.push_back((id, uid, pid));
        id
    }

    fn owner(&self, id: u32, uid: u32) -> Option<i32> {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.1.iter().find(|n| n.0 == id && n.1 == uid).map(|n| n.2)
    }
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
    notes: &Notes,
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
    let allowed = match call.need() {
        Need::Grant(cap) => grants.granted(&caller.app, cap),
        Need::Shower => grants.shows_notifications(&caller.app),
    };
    if !allowed {
        return Answer::err(
            Code::PermissionDenied,
            match call.need() {
                Need::Grant(_) => format!(
                    "Notifications are off for {}. Turn them on in System Settings > Privacy & Security.",
                    name(&caller.app)
                ),
                Need::Shower => "Only the desktop answers notifications.".into(),
            },
        );
    }
    match call {
        Call::NotificationPost {
            title,
            body,
            actions,
        } => {
            let id = notes.add(caller.uid, caller.pid);
            let e = Event::Notification {
                uid: caller.uid,
                app: caller.app.clone(),
                title,
                body,
                id,
                actions,
            };
            if bus.publish(&e) == 0 {
                return Answer::err(
                    Code::Unavailable,
                    "Notifications cannot be shown right now: the desktop is not running.",
                );
            }
            let mut a = Answer::ok();
            a.id = Some(id);
            a
        }
        Call::NotificationAnswer { id, key } => {
            let Some(pid) = notes.owner(id, caller.uid) else {
                return Answer::err(Code::InvalidArgument, "No such notification.");
            };
            let e = Event::NotificationAction {
                uid: caller.uid,
                pid,
                id,
                key,
            };
            if bus.publish(&e) == 0 {
                return Answer::err(
                    Code::Unavailable,
                    "The app that posted it is not listening.",
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
        fn shows_notifications(&self, app: &str) -> bool {
            app == "/usr/bin/wol-shell"
        }
    }

    const POST: &str = r#"{"call":"notification.post","title":"Done","body":"ok"}"#;

    fn caller(app: &str) -> Caller {
        Caller {
            uid: 1000,
            pid: 4242,
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
            bus.subscribe(Some(1000), None, s);
            let a = handle(json, who, &grants, &bus, &Notes::default());
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
        let a = handle(
            POST,
            Some(&bad),
            &grants,
            &Bus::<UnixStream>::default(),
            &Notes::default(),
        );
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
        bus.subscribe(Some(1000), None, s);
        let grants = Fixed(&[("/usr/bin/good", "notifications")]);
        let notes = Notes::default();
        let a = handle(POST, Some(&caller("/usr/bin/good")), &grants, &bus, &notes);
        assert_eq!(a.id, Some(1));
        let mut line = String::new();
        BufReader::new(p).read_line(&mut line).unwrap();
        assert_eq!(
            Event::parse(&line),
            Some(Event::Notification {
                uid: 1000,
                app: "/usr/bin/good".into(),
                title: "Done".into(),
                body: "ok".into(),
                id: 1,
                actions: Vec::new(),
            })
        );
        let nobody = Bus::<UnixStream>::default();
        let a = handle(
            POST,
            Some(&caller("/usr/bin/good")),
            &grants,
            &nobody,
            &notes,
        );
        assert_eq!(a.code, Some(Code::Unavailable));
    }

    #[test]
    fn a_pressed_button_reaches_only_the_poster() {
        let bus = Bus::default();
        let grants = Fixed(&[("/usr/bin/good", "notifications")]);
        let notes = Notes::default();
        let (shower, _s) = UnixStream::pair().unwrap();
        bus.subscribe(Some(1000), None, shower);
        let (app, app_peer) = UnixStream::pair().unwrap();
        bus.subscribe(None, Some((1000, 4242)), app);
        let (other, other_peer) = UnixStream::pair().unwrap();
        bus.subscribe(None, Some((1000, 9)), other);
        let post =
            r#"{"call":"notification.post","title":"Hi","actions":[{"key":"ok","label":"OK"}]}"#;
        let id = handle(post, Some(&caller("/usr/bin/good")), &grants, &bus, &notes)
            .id
            .unwrap();
        let answer = format!(r#"{{"call":"notification.answer","id":{id},"key":"ok"}}"#);
        let a = handle(
            &answer,
            Some(&caller("/usr/bin/good")),
            &grants,
            &bus,
            &notes,
        );
        assert_eq!(
            a.code,
            Some(Code::PermissionDenied),
            "apps cannot press buttons"
        );
        let mut shell = caller("/usr/bin/wol-shell");
        shell.pid = 1;
        assert!(handle(&answer, Some(&shell), &grants, &bus, &notes).ok);
        let mut line = String::new();
        BufReader::new(&app_peer).read_line(&mut line).unwrap();
        assert_eq!(
            Event::parse(&line),
            Some(Event::NotificationAction {
                uid: 1000,
                pid: 4242,
                id,
                key: "ok".into()
            })
        );
        other_peer.set_nonblocking(true).unwrap();
        let mut none = String::new();
        let _ = BufReader::new(&other_peer).read_line(&mut none);
        assert_eq!(none, "");
        shell.uid = 1001;
        let a = handle(&answer, Some(&shell), &grants, &bus, &notes);
        assert_eq!(
            a.code,
            Some(Code::InvalidArgument),
            "another account's note"
        );
    }

    #[test]
    fn notes_are_bounded() {
        let n = Notes::default();
        let first = n.add(1, 1);
        for _ in 0..NOTES_KEPT {
            n.add(1, 2);
        }
        assert_eq!(n.owner(first, 1), None);
        assert_eq!(n.owner(first + 1, 1), Some(2));
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
