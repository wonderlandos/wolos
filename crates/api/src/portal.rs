//! wold portal (ADR 0020): libwol service calls. The library is unprivileged and asks; wold decides.
//! Who asks comes from the kernel (peer uid, executable of the peer pid), never from the message.
//! Grants are the consent database the person controls in Settings > Privacy & Security.
use crate::bus::Bus;
use shared::events::Event;
use shared::portal::{Answer, Ask, Call, Code, Need};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

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
    /// Last id given, and the notes, newest last.
    inner: Mutex<(u32, VecDeque<Note>)>,
}

struct Note {
    id: u32,
    uid: u32,
    pid: i32,
    app: String,
}

pub const NOTES_KEPT: usize = 1024;

impl Notes {
    fn add(&self, c: &Caller) -> u32 {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.0 = g.0.wrapping_add(1).max(1);
        let id = g.0;
        if g.1.len() == NOTES_KEPT {
            g.1.pop_front();
        }
        g.1.push_back(Note {
            id,
            uid: c.uid,
            pid: c.pid,
            app: c.app.clone(),
        });
        id
    }

    fn owner(&self, id: u32, uid: u32) -> Option<i32> {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.1.iter()
            .find(|n| n.id == id && n.uid == uid)
            .map(|n| n.pid)
    }

    /// Did this very process post `id`? (Executable too: a pid can be reused.)
    fn posted_by(&self, id: u32, c: &Caller) -> bool {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.1.iter()
            .any(|n| n.id == id && n.uid == c.uid && n.pid == c.pid && n.app == c.app)
    }

    fn forget(&self, id: u32) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.1.retain(|n| n.id != id);
    }
}

/// How long a call waits for the desktop to answer (under libwol's own read timeout).
pub const ASK_TIMEOUT: Duration = Duration::from_secs(4);
/// Calls waiting for the desktop at once.
pub const ASKS_KEPT: usize = 64;
/// How long a file panel may stay open.
pub const PANEL_TIMEOUT: Duration = Duration::from_secs(600);

/// Calls waiting for the desktop: each [`Ask`] goes to the shell of the caller's account, and the
/// `session.answer` with its id wakes the call. An answer counts only from that account.
#[derive(Default)]
pub struct Asks {
    table: Mutex<AskTable>,
    answered: Condvar,
}

#[derive(Default)]
struct AskTable {
    last: u32,
    waiting: HashMap<u32, Waiting>,
}

struct Waiting {
    uid: u32,
    answer: Option<(Answer, Option<OwnedFd>)>,
}

impl Asks {
    fn lock(&self) -> std::sync::MutexGuard<'_, AskTable> {
        self.table.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Ask the desktop of `uid` and wait for it, at most `timeout`.
    pub fn ask<W: Write + Send>(
        &self,
        bus: &Bus<W>,
        uid: u32,
        ask: Ask,
        timeout: Duration,
    ) -> Answer {
        self.ask_for_file(bus, uid, ask, timeout).0
    }

    /// [`Asks::ask`], with the file the desktop attached to its answer.
    pub fn ask_for_file<W: Write + Send>(
        &self,
        bus: &Bus<W>,
        uid: u32,
        ask: Ask,
        timeout: Duration,
    ) -> (Answer, Option<OwnedFd>) {
        let id = {
            let mut t = self.lock();
            if t.waiting.len() >= ASKS_KEPT {
                return (
                    Answer::err(Code::Unavailable, "The desktop is busy. Try again."),
                    None,
                );
            }
            t.last = t.last.wrapping_add(1).max(1);
            let id = t.last;
            t.waiting.insert(id, Waiting { uid, answer: None });
            id
        };
        if bus.publish(&Event::SessionAsk { uid, id, ask }) == 0 {
            self.lock().waiting.remove(&id);
            return (
                Answer::err(Code::Unavailable, "The desktop is not running."),
                None,
            );
        }
        let end = Instant::now() + timeout;
        let mut t = self.lock();
        loop {
            if t.waiting.get(&id).is_some_and(|w| w.answer.is_some()) {
                break;
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                t.waiting.remove(&id);
                return (
                    Answer::err(Code::Unavailable, "The desktop did not answer."),
                    None,
                );
            }
            t = self
                .answered
                .wait_timeout(t, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        t.waiting
            .remove(&id)
            .and_then(|w| w.answer)
            .unwrap_or_else(|| {
                (
                    Answer::err(Code::Internal, "The desktop's answer was lost."),
                    None,
                )
            })
    }

    /// The desktop of `uid` answers ask `id`.
    fn answer(&self, uid: u32, id: u32, a: Answer, file: Option<OwnedFd>) -> Answer {
        let mut t = self.lock();
        let Some(w) = t
            .waiting
            .get_mut(&id)
            .filter(|w| w.uid == uid && w.answer.is_none())
        else {
            return Answer::err(Code::InvalidArgument, "Nothing is waiting for that answer.");
        };
        w.answer = Some((a, file));
        self.answered.notify_all();
        Answer::ok()
    }
}

/// What the portal keeps between calls.
#[derive(Default)]
pub struct State {
    pub notes: Notes,
    pub prefs: crate::prefs::Prefs,
    pub asks: Asks,
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

/// What a denied app was trying to do, for "<app> is not allowed to ...".
fn doing(capability: &str) -> Option<&'static str> {
    Some(match capability {
        "clipboard-read" => "read the clipboard",
        "clipboard-write" => "change the clipboard",
        "app-preferences" => "keep preferences",
        "file-panels" => "show file panels",
        _ => return None,
    })
}

fn denial(capability: &str, app: &str) -> String {
    let app = name(app);
    match doing(capability) {
        Some(what) => format!(
            "{app} is not allowed to {what}. Allow it in System Settings > Privacy & Security."
        ),
        None => format!(
            "Notifications are off for {app}. Turn them on in System Settings > Privacy & Security."
        ),
    }
}

/// What goes back besides the answer line.
#[derive(Default)]
pub struct Extra {
    /// Sent to the caller with the answer.
    pub file: Option<OwnedFd>,
}

fn panel<W: Write + Send>(
    st: &State,
    bus: &Bus<W>,
    caller: &Caller,
    extra: &mut Extra,
    save: bool,
    title: String,
    name: String,
) -> Answer {
    let ask = Ask::Panel { save, title, name };
    let (a, file) = st.asks.ask_for_file(bus, caller.uid, ask, PANEL_TIMEOUT);
    extra.file = file;
    a
}

/// [`respond`] for a call with nothing attached and nothing but an answer to send.
pub fn handle<W: Write + Send>(
    json: &str,
    caller: Option<&Caller>,
    grants: &dyn Grants,
    bus: &Bus<W>,
    st: &State,
) -> Answer {
    respond(json, caller, grants, bus, st, None).0
}

/// One call line's JSON -> answer. Nothing happens unless the call decodes, the caller is known
/// and the caller's executable holds the call's capability. `attached` is a descriptor that came
/// with the line; only an answer from the desktop uses it.
pub fn respond<W: Write + Send>(
    json: &str,
    caller: Option<&Caller>,
    grants: &dyn Grants,
    bus: &Bus<W>,
    st: &State,
    attached: Option<OwnedFd>,
) -> (Answer, Extra) {
    let mut extra = Extra::default();
    let a = decide(json, caller, grants, bus, st, attached, &mut extra);
    (a, extra)
}

fn decide<W: Write + Send>(
    json: &str,
    caller: Option<&Caller>,
    grants: &dyn Grants,
    bus: &Bus<W>,
    st: &State,
    attached: Option<OwnedFd>,
    extra: &mut Extra,
) -> Answer {
    let notes = &st.notes;
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
                Need::Grant(cap) => denial(cap, &caller.app),
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
            let id = notes.add(caller);
            let e = Event::Notification {
                uid: caller.uid,
                app: caller.app.clone(),
                title,
                body,
                id,
                actions,
                replace: false,
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
        Call::NotificationReplace {
            id,
            title,
            body,
            actions,
        } => {
            if !notes.posted_by(id, caller) {
                return Answer::err(Code::InvalidArgument, "No such notification.");
            }
            let e = Event::Notification {
                uid: caller.uid,
                app: caller.app.clone(),
                title,
                body,
                id,
                actions,
                replace: true,
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
        Call::NotificationWithdraw { id } => {
            if !notes.posted_by(id, caller) {
                return Answer::err(Code::InvalidArgument, "No such notification.");
            }
            notes.forget(id);
            bus.publish(&Event::NotificationWithdrawn {
                uid: caller.uid,
                id,
            });
            Answer::ok()
        }
        Call::ClipboardRead {} => st
            .asks
            .ask(bus, caller.uid, Ask::ClipboardRead, ASK_TIMEOUT),
        Call::ClipboardWrite { text } => {
            st.asks
                .ask(bus, caller.uid, Ask::ClipboardWrite { text }, ASK_TIMEOUT)
        }
        Call::PrefsGet { key } => st.prefs.get(caller.uid, &caller.app, &key),
        Call::PrefsSet { key, value } => st.prefs.set(caller.uid, &caller.app, key, value),
        Call::PrefsRemove { key } => st.prefs.remove(caller.uid, &caller.app, &key),
        Call::PrefsKeys {} => st.prefs.keys(caller.uid, &caller.app),
        Call::PanelOpen { title } => panel(st, bus, caller, extra, false, title, String::new()),
        Call::PanelSave { title, name } => panel(st, bus, caller, extra, true, title, name),
        Call::SessionAnswer { id, error, data } => {
            let a = match error {
                Some(m) => Answer::err(Code::Unavailable, m),
                None => data.map_or_else(Answer::ok, Answer::with),
            };
            st.asks.answer(caller.uid, id, a, attached)
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
    use std::sync::Arc;

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
            let a = handle(json, who, &grants, &bus, &State::default());
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
            &State::default(),
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
        let notes = State::default();
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
                replace: false,
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
        let notes = State::default();
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

    /// The next event on a subscriber, if any.
    fn next(p: &UnixStream) -> Option<Event> {
        p.set_nonblocking(true).unwrap();
        let mut line = String::new();
        BufReader::new(p).read_line(&mut line).ok()?;
        Event::parse(&line)
    }

    #[test]
    fn only_the_poster_replaces_or_withdraws() {
        let bus = Bus::default();
        let (shower, shown) = UnixStream::pair().unwrap();
        bus.subscribe(Some(1000), None, shower);
        let grants = Fixed(&[
            ("/usr/bin/good", "notifications"),
            ("/usr/bin/other", "notifications"),
            ("/usr/bin/nobody", "camera"),
        ]);
        let notes = State::default();
        let good = caller("/usr/bin/good");
        let run = |json: &str, who: &Caller| handle(json, Some(who), &grants, &bus, &notes);
        let id = run(POST, &good).id.unwrap();
        assert!(matches!(
            next(&shown),
            Some(Event::Notification { replace: false, .. })
        ));

        let replace = format!(r#"{{"call":"notification.replace","id":{id},"title":"New"}}"#);
        let withdraw = format!(r#"{{"call":"notification.withdraw","id":{id}}}"#);
        let mut other_pid = good.clone();
        other_pid.pid = 7;
        let mut other_uid = good.clone();
        other_uid.uid = 1001;
        let cases: [(&Caller, Code); 4] = [
            (&caller("/usr/bin/other"), Code::InvalidArgument),
            (&other_pid, Code::InvalidArgument),
            (&other_uid, Code::InvalidArgument),
            (&caller("/usr/bin/nobody"), Code::PermissionDenied),
        ];
        for (who, want) in cases {
            for json in [&replace, &withdraw] {
                assert_eq!(run(json, who).code, Some(want), "{json} {who:?}");
            }
        }
        assert_eq!(next(&shown), None, "nothing reached the desktop");

        let a = run(&replace, &good);
        assert_eq!((a.ok, a.id), (true, Some(id)));
        assert_eq!(
            next(&shown),
            Some(Event::Notification {
                uid: 1000,
                app: "/usr/bin/good".into(),
                title: "New".into(),
                body: String::new(),
                id,
                actions: Vec::new(),
                replace: true,
            })
        );
        assert!(run(&withdraw, &good).ok);
        assert_eq!(
            next(&shown),
            Some(Event::NotificationWithdrawn { uid: 1000, id })
        );
        assert_eq!(
            run(&withdraw, &good).code,
            Some(Code::InvalidArgument),
            "once"
        );
        assert_eq!(run(&replace, &good).code, Some(Code::InvalidArgument));
    }

    #[test]
    fn every_capability_is_grantable_refused_by_wold_and_explained() {
        for c in shared::portal::CAPABILITIES {
            assert!(wol_core::consent::CAPABILITIES.contains(c), "{c}");
            assert_eq!(
                wol_core::security::enforcement(c),
                Some(wol_core::security::Enforcement::System),
                "{c}"
            );
            assert!(*c == "notifications" || doing(c).is_some(), "{c}");
        }
    }

    /// A shell: answers its first ask with the line `f` makes, as account `uid`.
    fn shell(
        s: Arc<(Bus<UnixStream>, State)>,
        uid: u32,
        f: impl Fn(Ask) -> String + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        let (a, b) = UnixStream::pair().unwrap();
        s.0.subscribe(Some(uid), None, a);
        std::thread::spawn(move || {
            let mut r = BufReader::new(b);
            let mut line = String::new();
            while r.read_line(&mut line).is_ok_and(|n| n > 0) {
                if let Some(Event::SessionAsk { id, ask, .. }) = Event::parse(&line) {
                    let answer = f(ask).replace("ID", &id.to_string());
                    let mut me = caller("/usr/bin/wol-shell");
                    me.uid = uid;
                    handle(&answer, Some(&me), &Fixed(&[]), &s.0, &s.1);
                    return;
                }
                line.clear();
            }
        })
    }

    #[test]
    fn clipboard_goes_through_the_desktop_of_the_callers_account() {
        let s = Arc::new((Bus::<UnixStream>::default(), State::default()));
        let grants = Fixed(&[
            ("/usr/bin/reader", "clipboard-read"),
            ("/usr/bin/writer", "clipboard-write"),
        ]);
        let (reader, writer) = (caller("/usr/bin/reader"), caller("/usr/bin/writer"));
        let read = r#"{"call":"clipboard.read"}"#;
        let write = r#"{"call":"clipboard.write","text":"hi"}"#;
        let a = handle(write, Some(&reader), &grants, &s.0, &s.1);
        assert_eq!(a.code, Some(Code::PermissionDenied));
        assert_eq!(
            a.message.as_deref(),
            Some(
                "reader is not allowed to change the clipboard. Allow it in System Settings > \
                 Privacy & Security."
            )
        );
        assert_eq!(
            handle(read, Some(&writer), &grants, &s.0, &s.1).code,
            Some(Code::PermissionDenied)
        );
        assert_eq!(
            handle(read, Some(&reader), &grants, &s.0, &s.1).code,
            Some(Code::Unavailable),
            "no desktop"
        );
        let t = shell(s.clone(), 1000, |ask| {
            assert_eq!(ask, Ask::ClipboardRead);
            r#"{"call":"session.answer","id":ID,"data":{"text":"copied"}}"#.into()
        });
        let (other, seen) = UnixStream::pair().unwrap();
        s.0.subscribe(Some(1001), None, other);
        let a = handle(read, Some(&reader), &grants, &s.0, &s.1);
        assert_eq!(a.data, Some(serde_json::json!({"text": "copied"})));
        t.join().unwrap();
        assert_eq!(next(&seen), None, "another account's desktop saw nothing");
    }

    #[test]
    fn only_a_waiting_ask_of_that_account_takes_an_answer() {
        let s = Arc::new((Bus::<UnixStream>::default(), State::default()));
        let g = Fixed(&[("/usr/bin/reader", "clipboard-read")]);
        let forged = r#"{"call":"session.answer","id":1,"data":{"text":"x"}}"#;
        let desktop = caller("/usr/bin/wol-shell");
        let a = handle(forged, Some(&desktop), &g, &s.0, &s.1);
        assert_eq!(a.code, Some(Code::InvalidArgument), "nothing waits");
        let a = handle(forged, Some(&caller("/usr/bin/reader")), &g, &s.0, &s.1);
        assert_eq!(a.code, Some(Code::PermissionDenied), "apps cannot answer");

        let t = shell(s.clone(), 1000, |_| {
            r#"{"call":"session.answer","id":ID,"error":"The clipboard is empty."}"#.into()
        });
        let a = handle(
            r#"{"call":"clipboard.read"}"#,
            Some(&caller("/usr/bin/reader")),
            &g,
            &s.0,
            &s.1,
        );
        assert_eq!(a.code, Some(Code::Unavailable));
        assert_eq!(a.message.as_deref(), Some("The clipboard is empty."));
        t.join().unwrap();
        assert!(s.1.asks.lock().waiting.is_empty());

        let (a1, _b1) = UnixStream::pair().unwrap();
        s.0.subscribe(Some(1000), None, a1);
        let s2 = s.clone();
        let w = std::thread::spawn(move || {
            s2.1.asks
                .ask(&s2.0, 1000, Ask::ClipboardRead, Duration::from_secs(10))
        });
        let id = loop {
            if let Some(i) = s.1.asks.lock().waiting.keys().next() {
                break *i;
            }
            std::thread::yield_now();
        };
        let line = format!(r#"{{"call":"session.answer","id":{id},"data":{{"text":"z"}}}}"#);
        let mut stranger = desktop.clone();
        stranger.uid = 1001;
        assert_eq!(
            handle(&line, Some(&stranger), &g, &s.0, &s.1).code,
            Some(Code::InvalidArgument),
            "another account's desktop"
        );
        assert!(handle(&line, Some(&desktop), &g, &s.0, &s.1).ok);
        assert!(w.join().unwrap().ok);
    }

    #[test]
    fn an_ask_nobody_answers_times_out_and_is_forgotten() {
        let s = State::default();
        let bus = Bus::default();
        let (a, _b) = UnixStream::pair().unwrap();
        bus.subscribe(Some(1000), None, a);
        let got = s
            .asks
            .ask(&bus, 1000, Ask::ClipboardRead, Duration::from_millis(20));
        assert_eq!(got.code, Some(Code::Unavailable));
        assert!(s.asks.lock().waiting.is_empty());
    }

    #[test]
    fn prefs_need_the_grant_and_are_the_callers_own() {
        let root = std::env::temp_dir().join(format!("wol-portal-prefs-{}", std::process::id()));
        let st = State {
            prefs: crate::prefs::Prefs::at(&root),
            ..State::default()
        };
        let bus = Bus::<UnixStream>::default();
        let g = Fixed(&[
            ("/usr/bin/a", "app-preferences"),
            ("/usr/bin/b", "app-preferences"),
        ]);
        let run = |json: &str, app: &str| handle(json, Some(&caller(app)), &g, &bus, &st);
        let set = r#"{"call":"prefs.set","key":"k","value":"v"}"#;
        let get = r#"{"call":"prefs.get","key":"k"}"#;
        assert_eq!(run(set, "/usr/bin/c").code, Some(Code::PermissionDenied));
        assert!(run(set, "/usr/bin/a").ok);
        assert_eq!(
            run(get, "/usr/bin/a").data,
            Some(serde_json::json!({"value": "v"}))
        );
        assert_eq!(run(get, "/usr/bin/b").data, Some(serde_json::json!({})));
        assert_eq!(
            run(
                r#"{"call":"prefs.get","key":"../../etc/passwd"}"#,
                "/usr/bin/a"
            )
            .code,
            Some(Code::InvalidArgument)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn notes_are_bounded() {
        let n = Notes::default();
        let (a, b) = (
            Caller {
                uid: 1,
                pid: 1,
                app: "/a".into(),
            },
            Caller {
                uid: 1,
                pid: 2,
                app: "/a".into(),
            },
        );
        let first = n.add(&a);
        for _ in 0..NOTES_KEPT {
            n.add(&b);
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
