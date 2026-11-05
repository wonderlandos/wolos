// SPDX-License-Identifier: GPL-2.0-or-later
//! libwol 1.16 in wold: a player (grant media), its remote controls, and the three asks of the
//! desktop (recents, print, color-picker). wold decides with the kernel's identity; the desktop
//! alone presses keys, and only on the process that owns the player.
use api::bus::Bus;
use api::portal::{Caller, Grants, State, handle, respond};
use serde_json::json;
use shared::events::Event;
use shared::portal::{Ask, Code, MediaKey};
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

fn caller(app: &str) -> Caller {
    Caller {
        uid: 1000,
        pid: 4242,
        app: app.into(),
        scope: None,
    }
}

/// The next event on a subscriber, if any.
fn next(p: &UnixStream) -> Option<Event> {
    p.set_nonblocking(true).unwrap();
    let mut line = String::new();
    BufReader::new(p).read_line(&mut line).ok()?;
    Event::parse(&line)
}

const STATE: &str =
    r#"{"call":"media.state","state":{"status":"playing","title":"Song","position_us":1000000}}"#;
const PUBLISH: &str = r#"{"call":"media.publish"}"#;

#[test]
fn a_player_needs_its_grant_publishes_to_the_desktop_and_ends_with_the_connection() {
    let st = State::default();
    let bus = Bus::<UnixStream>::default();
    let (desktop, shown) = UnixStream::pair().unwrap();
    bus.subscribe(Some(1000), None, desktop);
    let g = Fixed(&[("/usr/bin/toy", "media")]);
    let toy = caller("/usr/bin/toy");
    // No grant, no player; nothing to the desktop.
    let nobody = caller("/usr/bin/nobody");
    let a = respond(PUBLISH, Some(&nobody), &g, &bus, &st, None).0;
    assert_eq!(a.code, Some(Code::PermissionDenied));
    assert!(!st.players.is_player(nobody.uid, nobody.pid));
    // A state before publishing is refused, even with the grant.
    let a = handle(STATE, Some(&toy), &g, &bus, &st);
    assert_eq!(a.code, Some(Code::InvalidArgument));
    assert_eq!(next(&shown), None);
    let (ans, extra) = respond(PUBLISH, Some(&toy), &g, &bus, &st, None);
    assert!(ans.ok);
    assert!(st.players.is_player(toy.uid, toy.pid));
    assert_eq!(
        extra.hold.as_ref().and_then(|h| h.gone.clone()),
        Some(Event::MediaGone {
            uid: 1000,
            pid: 4242
        }),
        "the end is announced once the connection closes"
    );
    let again = respond(PUBLISH, Some(&toy), &g, &bus, &st, None).0;
    assert_eq!(again.code, Some(Code::InvalidArgument), "one per process");
    assert!(handle(STATE, Some(&toy), &g, &bus, &st).ok);
    let Some(Event::MediaState {
        uid,
        pid,
        app,
        state,
    }) = next(&shown)
    else {
        panic!("the desktop was not told");
    };
    assert_eq!((uid, pid, app.as_str()), (1000, 4242, "/usr/bin/toy"));
    assert_eq!(state.title, "Song");
    drop(extra);
    assert!(!st.players.is_player(toy.uid, toy.pid));
    let a = handle(STATE, Some(&toy), &g, &bus, &st);
    assert_eq!(a.code, Some(Code::InvalidArgument), "gone is gone");
}

#[test]
fn the_app_cannot_name_itself_in_a_player_state() {
    // The name of the player is the kernel's: a state with an `app` field is not a call.
    let line = r#"{"call":"media.state","state":{"status":"playing","app":"/usr/bin/other"}}"#;
    let st = State::default();
    let bus = Bus::<UnixStream>::default();
    let g = Fixed(&[("/usr/bin/toy", "media")]);
    let a = handle(line, Some(&caller("/usr/bin/toy")), &g, &bus, &st);
    assert_eq!(a.code, Some(Code::InvalidArgument));
}

#[test]
fn only_the_desktop_presses_keys_and_only_on_the_owner() {
    let st = State::default();
    let bus = Bus::<UnixStream>::default();
    let g = Fixed(&[("/usr/bin/toy", "media")]);
    let toy = caller("/usr/bin/toy");
    let (ans, _held) = respond(PUBLISH, Some(&toy), &g, &bus, &st, None);
    assert!(ans.ok);
    let (mine, mine_seen) = UnixStream::pair().unwrap();
    bus.subscribe(None, Some((1000, 4242)), mine);
    let (other, other_seen) = UnixStream::pair().unwrap();
    bus.subscribe(None, Some((1000, 9)), other);
    let (stranger, stranger_seen) = UnixStream::pair().unwrap();
    bus.subscribe(None, Some((1001, 4242)), stranger);
    let key = r#"{"call":"media.key","player":4242,"key":"next"}"#;
    // An app (even the owner, even one with the grant) is not the desktop.
    for who in [&toy, &caller("/usr/bin/other")] {
        let a = handle(key, Some(who), &g, &bus, &st);
        assert_eq!(a.code, Some(Code::PermissionDenied), "{who:?}");
    }
    assert_eq!(next(&mine_seen), None);
    let shell = caller("/usr/bin/wol-shell");
    let a = handle(key, Some(&shell), &g, &bus, &st);
    assert!(a.ok, "{a:?}");
    assert_eq!(
        next(&mine_seen),
        Some(Event::MediaKey {
            uid: 1000,
            pid: 4242,
            key: MediaKey::Next,
            value_us: 0
        })
    );
    assert_eq!(next(&other_seen), None, "another process of the account");
    assert_eq!(next(&stranger_seen), None, "another account");
    // A process that publishes nothing gets no key; another account's player is not reachable.
    let gone = r#"{"call":"media.key","player":9,"key":"next"}"#;
    let a = handle(gone, Some(&shell), &g, &bus, &st);
    assert_eq!(a.code, Some(Code::InvalidArgument));
    let mut theirs = shell.clone();
    theirs.uid = 1001;
    let a = handle(key, Some(&theirs), &g, &bus, &st);
    assert_eq!(a.code, Some(Code::InvalidArgument));
    assert_eq!(next(&other_seen), None);
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
fn recents_colors_and_printing_ask_the_desktop_of_the_callers_account() {
    let s = Arc::new((Bus::<UnixStream>::default(), State::default()));
    let g = Fixed(&[
        ("/usr/bin/doc", "recents"),
        ("/usr/bin/doc", "print"),
        ("/usr/bin/paint", "color-picker"),
    ]);
    let (doc, paint, nobody) = (
        caller("/usr/bin/doc"),
        caller("/usr/bin/paint"),
        caller("/usr/bin/nobody"),
    );
    let recent = r#"{"call":"recent.add","path":"/home/a/b.pdf"}"#;
    let print = r#"{"call":"print.file","path":"/home/a/b.pdf"}"#;
    let pick = r#"{"call":"color.pick"}"#;
    for (call, cap) in [
        (recent, "recents"),
        (print, "print"),
        (pick, "color-picker"),
    ] {
        let a = handle(call, Some(&nobody), &g, &s.0, &s.1);
        assert_eq!(a.code, Some(Code::PermissionDenied), "{call}");
        let said = a.message.unwrap();
        assert!(
            said.contains("Allow it in System Settings"),
            "{cap}: {said}"
        );
    }
    let a = handle(pick, Some(&doc), &g, &s.0, &s.1);
    assert_eq!(
        a.code,
        Some(Code::PermissionDenied),
        "one grant is not another"
    );
    let bad = r#"{"call":"recent.add","path":"b.pdf"}"#;
    let a = handle(bad, Some(&doc), &g, &s.0, &s.1);
    assert_eq!(a.code, Some(Code::InvalidArgument));
    let (other, seen) = UnixStream::pair().unwrap();
    s.0.subscribe(Some(1001), None, other);
    let cases = [
        (
            recent,
            &doc,
            // The app is named by the kernel's word, never the call's.
            Ask::RecentAdd {
                path: "/home/a/b.pdf".into(),
                app: "/usr/bin/doc".into(),
            },
            json!({}),
        ),
        (
            print,
            &doc,
            Ask::PrintFile {
                path: "/home/a/b.pdf".into(),
            },
            json!({"done": "printed"}),
        ),
        (pick, &paint, Ask::ColorPick, json!({"color": "#336699"})),
    ];
    for (call, who, want, data) in cases {
        let reply = format!(r#"{{"call":"session.answer","id":ID,"data":{data}}}"#);
        let t = shell(s.clone(), 1000, move |ask| {
            assert_eq!(ask, want);
            reply.clone()
        });
        let a = handle(call, Some(who), &g, &s.0, &s.1);
        assert_eq!(a.data, Some(data), "{call}");
        t.join().unwrap();
    }
    assert_eq!(next(&seen), None, "another account's desktop saw nothing");
}
