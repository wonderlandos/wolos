// SPDX-License-Identifier: GPL-2.0-or-later
//! wold portal (ADR 0020): libwol service calls. The library is unprivileged and asks; wold decides.
//! Who asks comes from the kernel (peer uid, executable of the peer pid), never from the message.
//! Grants are the consent database the person controls in Settings > Privacy & Security.
use crate::bus::Bus;
use shared::events::Event;
use shared::portal::{Answer, Ask, Call, Code, Need};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use wol_core::system::Host;

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
/// Calls waiting for the desktop at once, all accounts together.
pub const ASKS_KEPT: usize = 64;
/// Calls of one account waiting at once (R2C-9): one account, or its frozen shell, never takes
/// the whole table.
pub const ASKS_PER_UID: usize = 8;
/// Approval sheets (package and system changes) of one account waiting at once (R2C-9).
pub const APPROVALS_PER_UID: usize = 2;
/// Who a call counts as for [`ASKS_PER_APP`]: the executable, except for web app hosts, which all
/// run `/usr/bin/wol-webhost` and are told apart by their app scope (`cgroup` reads a process's
/// cgroup text): one parked web app must not use up the others' places.
fn app_key(caller: &Caller, cgroup: &dyn Fn(i32) -> Option<String>) -> String {
    if caller.app == crate::webapp::HOST
        && let Some(id) =
            cgroup(caller.pid).and_then(|text| wol_core::sysapp::app_of_cgroup(&text, caller.pid))
    {
        return format!("webapp:{id}");
    }
    caller.app.clone()
}

fn cgroup_of(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()
}

/// Calls of one app (one executable of one account) waiting at once (QA-100 API-6): a file panel
/// or share sheet waits up to [`PANEL_TIMEOUT`], so one app parking asks never fills
/// [`ASKS_PER_UID`] for the account's other apps.
pub const ASKS_PER_APP: usize = 2;
pub use shared::portal::{BUSY, NO_ANSWER, NOT_RUNNING};
/// How long a file panel may stay open.
pub const PANEL_TIMEOUT: Duration = Duration::from_secs(600);
/// How long a search waits for the apps that provide rows.
pub const PROVIDER_WAIT: Duration = Duration::from_millis(200);
/// How long a message waits for its reply; libwol's own wait for wold is longer.
pub const MESSAGE_WAIT: Duration = Duration::from_secs(3);

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
    /// The asking app's executable, counted against [`ASKS_PER_APP`]; None for wold's own asks.
    app: Option<String>,
    /// An approval sheet ([`Ask::Approval`]), counted against [`APPROVALS_PER_UID`].
    approval: bool,
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
        self.ask_inner(bus, uid, None, ask, timeout)
    }

    /// [`Asks::ask`] for an app's call: also held to [`ASKS_PER_APP`].
    pub fn ask_from<W: Write + Send>(
        &self,
        bus: &Bus<W>,
        caller: &Caller,
        ask: Ask,
        timeout: Duration,
    ) -> Answer {
        let key = app_key(caller, &cgroup_of);
        self.ask_inner(bus, caller.uid, Some(&key), ask, timeout).0
    }

    /// [`Asks::ask_from`], with the file the desktop attached to its answer.
    pub fn ask_for_file_from<W: Write + Send>(
        &self,
        bus: &Bus<W>,
        caller: &Caller,
        ask: Ask,
        timeout: Duration,
    ) -> (Answer, Option<OwnedFd>) {
        let key = app_key(caller, &cgroup_of);
        self.ask_inner(bus, caller.uid, Some(&key), ask, timeout)
    }

    fn ask_inner<W: Write + Send>(
        &self,
        bus: &Bus<W>,
        uid: u32,
        app: Option<&str>,
        ask: Ask,
        timeout: Duration,
    ) -> (Answer, Option<OwnedFd>) {
        let approval = matches!(ask, Ask::Approval { .. });
        let id = {
            let mut t = self.lock();
            let mine = t.waiting.values().filter(|w| w.uid == uid);
            let (n, sheets) = mine.fold((0, 0), |(n, a), w| (n + 1, a + usize::from(w.approval)));
            let of_app = app.map_or(0, |a| {
                t.waiting
                    .values()
                    .filter(|w| w.uid == uid && w.app.as_deref() == Some(a))
                    .count()
            });
            if t.waiting.len() >= ASKS_KEPT
                || n >= ASKS_PER_UID
                || of_app >= ASKS_PER_APP
                || (approval && sheets >= APPROVALS_PER_UID)
            {
                return (Answer::err(Code::Unavailable, BUSY), None);
            }
            // Skip ids still waiting (after a wrap), so an answer never reaches the wrong call.
            loop {
                t.last = t.last.wrapping_add(1).max(1);
                if !t.waiting.contains_key(&t.last) {
                    break;
                }
            }
            let id = t.last;
            t.waiting.insert(
                id,
                Waiting {
                    uid,
                    app: app.map(str::to_owned),
                    approval,
                    answer: None,
                },
            );
            id
        };
        if bus.publish(&Event::SessionAsk { uid, id, ask }) == 0 {
            self.lock().waiting.remove(&id);
            return (Answer::err(Code::Unavailable, NOT_RUNNING), None);
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
                return (Answer::err(Code::Unavailable, NO_ANSWER), None);
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

/// Connections holding something, per account.
#[derive(Default)]
pub struct Holds(Mutex<HashMap<u32, usize>>);

impl Holds {
    fn take(&self, uid: u32) -> bool {
        let mut g = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let n = g.entry(uid).or_default();
        if *n >= shared::portal::MAX_HOLDS {
            return false;
        }
        *n += 1;
        true
    }

    /// The connection of a [`Held`] closed.
    pub fn give(&self, uid: u32) {
        let mut g = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(n) = g.get_mut(&uid) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                g.remove(&uid);
            }
        }
    }
}

/// Lives as long as the caller's connection.
pub struct Held {
    pub uid: u32,
    _guard: Box<dyn Send>,
}

/// What the portal keeps between calls.
pub struct State {
    pub notes: Notes,
    pub prefs: crate::prefs::Prefs,
    pub searches: crate::search::Searches,
    pub endpoints: crate::ipc::Endpoints,
    pub background: Box<dyn crate::services::Background>,
    pub asks: Asks,
    pub holds: Holds,
    pub host: Box<dyn Host>,
    pub inhibit: Box<dyn crate::facts::Inhibit>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            prefs: Default::default(),
            searches: Default::default(),
            background: Box::new(crate::services::NoBackground),
            endpoints: Default::default(),
            notes: Default::default(),
            asks: Default::default(),
            holds: Default::default(),
            // Audit S14: commands wold runs for facts and devices get the cleared environment.
            host: Box::new(crate::privileged::PrivilegedHost),
            inhibit: Box::new(crate::facts::Logind),
        }
    }
}

pub trait Grants {
    fn granted(&self, app: &str, capability: &str) -> bool;
    /// May receive the account's notifications (`subscribe account`).
    fn shows_notifications(&self, app: &str) -> bool;
    /// `app` of account `uid` was refused `capability`. Nothing by default; wold remembers it
    /// ([`Remembering`]) so Settings can show the app with its switch off.
    fn refused(&self, uid: u32, app: &str, capability: &str) {
        let _ = (uid, app, capability);
    }
}

/// [`Grants`] that remember each refusal in the account's asked file under `dir`
/// (`wol_core::consent::ASKED_DIR`): no access changes, the refused app only becomes visible in
/// Settings > Privacy & Security, where the refusal sends the person (MANUAL 0.12.0: "Notifications
/// are off for hello" with no switch for hello anywhere). Best effort: a write that fails goes to
/// `log` (the binary's choice) and the call is answered as before.
pub struct Remembering<G> {
    pub grants: G,
    pub dir: PathBuf,
    pub log: fn(&str),
}

impl<G: Grants> Grants for Remembering<G> {
    fn granted(&self, app: &str, capability: &str) -> bool {
        self.grants.granted(app, capability)
    }
    fn shows_notifications(&self, app: &str) -> bool {
        self.grants.shows_notifications(app)
    }
    fn refused(&self, uid: u32, app: &str, capability: &str) {
        if let Err(e) = remember_refusal(&self.dir, uid, app, capability) {
            (self.log)(&format!(
                "could not note that {app} asked for {capability}: {e}"
            ));
        }
    }
}

/// Adds (app, capability) to `<dir>/<uid>.json` unless it is listed already. The file belongs to
/// the account (0600, so no other account learns which apps it runs); it is read bounded, since
/// the account could grow it, and replaced whole by root. The directory is root's (0755).
pub fn remember_refusal(dir: &Path, uid: u32, app: &str, capability: &str) -> std::io::Result<()> {
    use std::io::Read;
    use std::os::unix::fs::DirBuilderExt;
    let path = dir.join(format!("{uid}.json"));
    let mut cur = String::new();
    if let Ok(f) = std::fs::File::open(&path) {
        let _ = f
            .take(wol_core::consent::MAX_ASKED_BYTES)
            .read_to_string(&mut cur);
    }
    let Some(text) = wol_core::consent::note_asked(Some(&cur), app, capability) else {
        return Ok(());
    };
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(dir)?;
    shared::atomic::write_as(&path, 0o600, Some(uid), text.as_bytes())
}

/// Programs that show notifications: the shell.
pub const SHOWERS: [&str; 1] = ["/usr/bin/wol-shell"];
/// The one program that collects apps' rows, and so shows them to the person.
pub const SEARCHBAR: &str = "/usr/bin/wol-search";

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
        "power-info" => "see the battery",
        "keep-awake" => "keep this computer awake",
        "network-info" => "see the network state",
        "audio-info" => "see the volume and what is playing",
        "audio-control" => "change the volume",
        "search" => "search",
        "search-provider" => "add search results",
        "account-info" => "read your account name",
        "ipc" => "send or receive messages from other apps",
        "devices" => "use connected devices",
        "share" => "share text and files",
        "trash" => "move files to the Trash",
        "background" => "run in the background",
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
    /// Kept until the caller's connection closes.
    pub hold: Option<Held>,
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
    let (a, file) = st.asks.ask_for_file_from(bus, caller, ask, PANEL_TIMEOUT);
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
        Need::Search => caller.app == SEARCHBAR,
        Need::Device => call.device().is_some_and(|(v, p)| {
            grants.granted(&caller.app, &wol_core::consent::device_grant(v, p))
        }),
        Need::Peer => call
            .peer()
            .is_some_and(|p| grants.granted(&caller.app, &wol_core::consent::peer_grant(p))),
        Need::HelperGate => caller.app == shared::portal::HELPER_GATE && caller.uid != 0,
    };
    if !allowed {
        if let Need::Grant(cap) = call.need() {
            grants.refused(caller.uid, &caller.app, cap);
        }
        return Answer::err(
            Code::PermissionDenied,
            match call.need() {
                Need::Grant(cap) => denial(cap, &caller.app),
                Need::Shower => "Only the desktop answers notifications.".into(),
                Need::Search => "Only Search collects search results.".into(),
                Need::Peer => denial("ipc", &caller.app),
                Need::Device => denial("devices", &caller.app),
                Need::HelperGate => {
                    "Only the WolOS helper gate asks for administrator changes.".into()
                }
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
            .ask_from(bus, caller, Ask::ClipboardRead, ASK_TIMEOUT),
        Call::ClipboardWrite { text } => {
            st.asks
                .ask_from(bus, caller, Ask::ClipboardWrite { text }, ASK_TIMEOUT)
        }
        Call::PrefsGet { key } => st.prefs.get(caller.uid, &caller.app, &key),
        Call::PrefsSet { key, value } => st.prefs.set(caller.uid, &caller.app, key, value),
        Call::PrefsRemove { key } => st.prefs.remove(caller.uid, &caller.app, &key),
        Call::PrefsKeys {} => st.prefs.keys(caller.uid, &caller.app),
        Call::PanelOpen { title } => panel(st, bus, caller, extra, false, title, String::new()),
        Call::PanelSave { title, name } => panel(st, bus, caller, extra, true, title, name),
        Call::PowerInfo {} => Answer::with(crate::facts::power(&*st.host)),
        Call::PowerKeepAwake { reason } => {
            let refused = |m: &str| Answer::err(Code::Unavailable, m);
            if !st.holds.take(caller.uid) {
                return refused("Too many apps are keeping this computer awake.");
            }
            match st.inhibit.hold(name(&caller.app), &reason) {
                Some(guard) => {
                    extra.hold = Some(Held {
                        uid: caller.uid,
                        _guard: guard,
                    });
                    Answer::ok()
                }
                None => {
                    st.holds.give(caller.uid);
                    refused("WolOS could not keep this computer awake.")
                }
            }
        }
        Call::NetworkInfo {} => Answer::with(crate::facts::network(&*st.host)),
        Call::AudioInfo {} => st.asks.ask_from(bus, caller, Ask::AudioInfo, ASK_TIMEOUT),
        Call::AudioSetVolume { percent } => {
            let ask = Ask::AudioSetVolume { percent };
            st.asks.ask_from(bus, caller, ask, ASK_TIMEOUT)
        }
        Call::AudioSetMuted { muted } => {
            let ask = Ask::AudioSetMuted { muted };
            st.asks.ask_from(bus, caller, ask, ASK_TIMEOUT)
        }
        Call::SearchQuery { text } => {
            st.asks
                .ask_from(bus, caller, Ask::Search { text }, ASK_TIMEOUT)
        }
        Call::SearchProvide {} => {
            if !st.holds.take(caller.uid) {
                return Answer::err(Code::Unavailable, "Too many search providers are running.");
            }
            extra.hold = Some(Held {
                uid: caller.uid,
                _guard: Box::new(st.searches.register(caller)),
            });
            Answer::ok()
        }
        Call::SearchReply { id, results } => {
            if st.searches.reply(caller, id, results) {
                Answer::ok()
            } else {
                Answer::err(Code::InvalidArgument, "Nothing is waiting for that reply.")
            }
        }
        Call::SearchCollect { text } => {
            let rows = st.searches.collect(bus, caller.uid, &text, PROVIDER_WAIT);
            Answer::with(serde_json::json!({ "results": rows }))
        }
        Call::SearchOpen { provider, key } => {
            if !st.searches.is_provider(caller.uid, provider) {
                return Answer::err(Code::InvalidArgument, "That app stopped providing results.");
            }
            bus.publish(&Event::SearchOpen {
                uid: caller.uid,
                pid: provider,
                key,
            });
            Answer::ok()
        }
        Call::AccountInfo {} => std::fs::read_to_string("/etc/passwd")
            .ok()
            .and_then(|t| crate::facts::account(&t, caller.uid))
            .map_or_else(
                || Answer::err(Code::Unavailable, "This account has no name."),
                Answer::with,
            ),
        Call::IpcListen { name } => {
            if !st.holds.take(caller.uid) {
                return Answer::err(Code::Unavailable, "Too many apps are listening.");
            }
            let Some(l) = st.endpoints.listen(caller, &name) else {
                st.holds.give(caller.uid);
                return Answer::err(Code::InvalidArgument, "This app already listens there.");
            };
            extra.hold = Some(Held {
                uid: caller.uid,
                _guard: Box::new(l),
            });
            Answer::ok()
        }
        Call::IpcSend { peer, name, body } => {
            let to = (peer.as_str(), name.as_str());
            match st.endpoints.send(bus, caller, to, body, MESSAGE_WAIT) {
                Ok(body) => Answer::with(serde_json::json!({ "body": body })),
                Err(m) => Answer::err(Code::Unavailable, m),
            }
        }
        Call::IpcReply { id, body } => {
            if st.endpoints.reply(caller, id, body) {
                Answer::ok()
            } else {
                Answer::err(Code::InvalidArgument, "Nothing is waiting for that reply.")
            }
        }
        Call::IpcPublish { topic, body } => {
            let grant = wol_core::consent::peer_grant(&caller.app);
            let hears = |app: &str| grants.granted(app, &grant);
            st.endpoints.publish(bus, caller, &topic, &body, hears);
            Answer::ok()
        }
        Call::DeviceList {} => Answer::with(crate::devices::list(
            Path::new(crate::devices::USB_SYSFS),
            &*st.host,
        )),
        Call::DeviceOpen { vendor, product } => {
            let (sys, nodes) = (crate::devices::USB_SYSFS, crate::devices::USB_NODES);
            match crate::devices::open(Path::new(sys), Path::new(nodes), vendor, product) {
                Ok((name, file)) => {
                    extra.file = Some(file.into());
                    Answer::with(serde_json::json!({ "name": name }))
                }
                Err(m) => Answer::err(Code::Unavailable, m),
            }
        }
        Call::ShareText { title, text } => {
            let ask = Ask::Share { title, text };
            st.asks.ask_from(bus, caller, ask, PANEL_TIMEOUT)
        }
        // The desktop acts as the person: wold never opens or moves a path for an app.
        Call::FileTrash { path } => st
            .asks
            .ask_from(bus, caller, Ask::Trash { path }, ASK_TIMEOUT),
        Call::ShareFile { path } => {
            st.asks
                .ask_from(bus, caller, Ask::ShareFile { path }, PANEL_TIMEOUT)
        }
        Call::OpenWith { path } => {
            st.asks
                .ask_from(bus, caller, Ask::OpenWith { path }, PANEL_TIMEOUT)
        }
        Call::ServiceStart { name } => match st.background.start(caller, &name) {
            Ok(()) => Answer::ok(),
            Err(m) => Answer::err(Code::Unavailable, m),
        },
        Call::ServiceStop { name } => {
            if st.background.stop(caller, &name) {
                Answer::ok()
            } else {
                Answer::err(Code::InvalidArgument, "This app runs no such service.")
            }
        }
        Call::ServiceStatus { name } => Answer::with(st.background.status(caller, &name)),
        Call::SessionAnswer { id, error, data } => {
            let a = match error {
                Some(m) => Answer::err(Code::Unavailable, m),
                None => data.map_or_else(Answer::ok, Answer::with),
            };
            st.asks.answer(caller.uid, id, a, attached)
        }
        Call::SessionApprove { approval } => {
            // R2C-6: the sheet names the caller by the executable the kernel reported; the name
            // the helper gave itself only goes to the audit text. The shell runs the change
            // through wol-admind after the password and answers Done or Failed.
            let approval = shared::shell::Approval {
                tool: format!("{}: {}", approval.helper, approval.tool),
                helper: caller.app.clone(),
                request: approval.request,
            };
            let wait = Duration::from_secs(shared::shell::APPROVAL_WAIT_SECS.into());
            st.asks
                .ask_from(bus, caller, Ask::Approval { approval }, wait)
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
    fn file_calls_reach_only_the_desktop_of_the_callers_account() {
        let s = Arc::new((Bus::<UnixStream>::default(), State::default()));
        let grants = Fixed(&[("/usr/bin/sharer", "share"), ("/usr/bin/tidy", "trash")]);
        let (sharer, tidy) = (caller("/usr/bin/sharer"), caller("/usr/bin/tidy"));
        let trash = r#"{"call":"file.trash","path":"/home/a/b.jpg"}"#;
        let share = r#"{"call":"share.file","path":"/home/a/b.jpg"}"#;
        let open = r#"{"call":"open.with","path":"/home/a/b.jpg"}"#;
        let a = handle(trash, Some(&sharer), &grants, &s.0, &s.1);
        assert_eq!(a.code, Some(Code::PermissionDenied));
        assert_eq!(
            a.message.as_deref(),
            Some(
                "sharer is not allowed to move files to the Trash. Allow it in System Settings > \
                 Privacy & Security."
            )
        );
        for c in [share, open] {
            let a = handle(c, Some(&tidy), &grants, &s.0, &s.1);
            assert_eq!(a.code, Some(Code::PermissionDenied), "{c}");
        }
        let a = handle(
            r#"{"call":"file.trash","path":"/home/a/../b"}"#,
            Some(&tidy),
            &grants,
            &s.0,
            &s.1,
        );
        assert_eq!(a.code, Some(Code::InvalidArgument), "not a normal path");
        let (other, seen) = UnixStream::pair().unwrap();
        s.0.subscribe(Some(1001), None, other);
        let cases = [
            (
                trash,
                &tidy,
                Ask::Trash {
                    path: "/home/a/b.jpg".into(),
                },
                r#"{"name":"b.jpg"}"#,
            ),
            (
                share,
                &sharer,
                Ask::ShareFile {
                    path: "/home/a/b.jpg".into(),
                },
                r#"{"done":"copied"}"#,
            ),
            (
                open,
                &sharer,
                Ask::OpenWith {
                    path: "/home/a/b.jpg".into(),
                },
                r#"{"done":"opened"}"#,
            ),
        ];
        for (call, who, want, data) in cases {
            let reply = format!(r#"{{"call":"session.answer","id":ID,"data":{data}}}"#);
            let t = shell(s.clone(), 1000, move |ask| {
                assert_eq!(ask, want);
                reply.clone()
            });
            let a = handle(call, Some(who), &grants, &s.0, &s.1);
            assert_eq!(a.data, serde_json::from_str(data).ok(), "{call}");
            t.join().unwrap();
        }
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

    /// R2C-6: a helper's administrator change reaches the caller's own shower as an approval
    /// that names the helper gate by its kernel executable, and the sheet's answer comes back.
    /// Only the helper gate may send it, only admin kinds, and only the shower answers it: a
    /// same-account impostor's `session.answer` is refused and changes nothing.
    #[test]
    fn a_helper_admin_change_is_asked_through_wold() {
        let s = Arc::new((Bus::<UnixStream>::default(), State::default()));
        let g = Fixed(&[]);
        let line = r#"{"call":"session.approve","approval":{"helper":"Claude Desktop","tool":"wifi off","request":{"action":"wifi","on":false}}}"#;
        // Not the helper gate: refused before anything is shown.
        let a = handle(line, Some(&caller("/usr/bin/python3")), &g, &s.0, &s.1);
        assert_eq!(a.code, Some(Code::PermissionDenied));
        let mut root = caller(shared::portal::HELPER_GATE);
        root.uid = 0;
        assert_eq!(
            handle(line, Some(&root), &g, &s.0, &s.1).code,
            Some(Code::PermissionDenied)
        );
        // A user-level change or a package change is not relayed.
        for bad in [
            r#"{"call":"session.approve","approval":{"helper":"h","tool":"t","request":{"action":"change","capability":"appearance","what":"x"}}}"#,
            r#"{"call":"session.approve","approval":{"helper":"h","tool":"t","request":{"action":"packages","verb":"install","names":["vim"]}}}"#,
            r#"{"call":"session.approve","approval":{"helper":"h
x","tool":"t","request":{"action":"wifi","on":true}}}"#,
        ] {
            let a = handle(
                bad,
                Some(&caller(shared::portal::HELPER_GATE)),
                &g,
                &s.0,
                &s.1,
            );
            assert_eq!(a.code, Some(Code::InvalidArgument), "{bad}");
        }
        assert!(s.1.asks.lock().waiting.is_empty());

        let (mine, seen) = UnixStream::pair().unwrap();
        assert!(s.0.subscribe(Some(1000), Some((1000, 1)), mine));
        let s2 = s.clone();
        let helper = std::thread::spawn(move || {
            handle(
                line,
                Some(&caller(shared::portal::HELPER_GATE)),
                &Fixed(&[]),
                &s2.0,
                &s2.1,
            )
        });
        let mut got = String::new();
        BufReader::new(&seen).read_line(&mut got).unwrap();
        let Some(Event::SessionAsk {
            uid: 1000,
            id,
            ask: Ask::Approval { approval },
        }) = Event::parse(got.trim())
        else {
            panic!("{got}");
        };
        assert_eq!(
            approval.helper,
            shared::portal::HELPER_GATE,
            "kernel executable"
        );
        assert_eq!(approval.tool, "Claude Desktop: wifi off");
        assert_eq!(
            approval.request,
            shared::shell::ApprovalRequest::Wifi { on: false }
        );
        // A same-account impostor (not the shell's executable) cannot answer it.
        let done = format!(r#"{{"call":"session.answer","id":{id},"data":{{"answer":"done"}}}}"#);
        let a = handle(&done, Some(&caller("/usr/bin/python3")), &g, &s.0, &s.1);
        assert_eq!(a.code, Some(Code::PermissionDenied));
        assert!(
            s.1.asks
                .lock()
                .waiting
                .get(&id)
                .is_some_and(|w| w.answer.is_none())
        );
        // The shower of another account cannot either.
        let mut other = caller(SHOWERS[0]);
        other.uid = 1001;
        assert_eq!(
            handle(&done, Some(&other), &g, &s.0, &s.1).code,
            Some(Code::InvalidArgument)
        );
        // The account's shower answers; the helper hears the sheet's answer.
        assert!(handle(&done, Some(&caller(SHOWERS[0])), &g, &s.0, &s.1).ok);
        let a = helper.join().unwrap();
        assert!(a.ok, "{a:?}");
        assert_eq!(a.data, Some(serde_json::json!({"answer": "done"})));
    }

    /// R2C-9: one account's unanswered asks fill only its own share; another account still asks.
    #[test]
    fn one_account_cannot_starve_another() {
        let s = Arc::new((Bus::<UnixStream>::default(), State::default()));
        let mut keep = Vec::new();
        for uid in [1000, 1001] {
            let (a, b) = UnixStream::pair().unwrap();
            assert!(s.0.subscribe(Some(uid), None, a));
            keep.push(b);
        }
        let wait = Duration::from_secs(5);
        let waiters: Vec<_> = (0..ASKS_PER_UID)
            .map(|_| {
                let s = s.clone();
                std::thread::spawn(move || s.1.asks.ask(&s.0, 1000, Ask::ClipboardRead, wait))
            })
            .collect();
        while s.1.asks.lock().waiting.len() < ASKS_PER_UID {
            std::thread::yield_now();
        }
        let ninth = s.1.asks.ask(&s.0, 1000, Ask::ClipboardRead, wait);
        assert_eq!(ninth.message.as_deref(), Some(BUSY));
        // uid 1001 is still admitted: it waits (and times out) instead of hearing "busy".
        let other =
            s.1.asks
                .ask(&s.0, 1001, Ask::ClipboardRead, Duration::from_millis(50));
        assert_eq!(other.message.as_deref(), Some(NO_ANSWER));
        // Approval sheets have their own, smaller share.
        let approval = || Ask::Approval {
            approval: shared::shell::Approval {
                helper: "/usr/bin/x".into(),
                tool: "t".into(),
                request: shared::shell::ApprovalRequest::Change {
                    capability: "packages-install".into(),
                    what: "x".into(),
                },
            },
        };
        let sheets: Vec<_> = (0..APPROVALS_PER_UID)
            .map(|_| {
                let (s, a) = (s.clone(), approval());
                std::thread::spawn(move || s.1.asks.ask(&s.0, 1001, a, wait))
            })
            .collect();
        while s.1.asks.lock().waiting.len() < ASKS_PER_UID + APPROVALS_PER_UID {
            std::thread::yield_now();
        }
        let third = s.1.asks.ask(&s.0, 1001, approval(), wait);
        assert_eq!(third.message.as_deref(), Some(BUSY));
        for w in waiters.into_iter().chain(sheets) {
            assert_eq!(w.join().unwrap().message.as_deref(), Some(NO_ANSWER));
        }
        assert!(s.1.asks.lock().waiting.is_empty());
    }

    /// QA-100 API-6: one share-granted app parking file panels fills only its own
    /// [`ASKS_PER_APP`]; the account's other apps still ask.
    #[test]
    fn per_app_ask_cap() {
        let s = Arc::new((Bus::<UnixStream>::default(), State::default()));
        let (a, _keep) = UnixStream::pair().unwrap();
        assert!(s.0.subscribe(Some(1000), None, a));
        let greedy = Caller {
            uid: 1000,
            pid: 10,
            app: "/usr/bin/greedy".into(),
        };
        let wait = Duration::from_secs(5);
        let parked: Vec<_> = (0..ASKS_PER_APP)
            .map(|_| {
                let (s, c) = (s.clone(), greedy.clone());
                std::thread::spawn(move || {
                    s.1.asks
                        .ask_from(&s.0, &c, Ask::ShareFile { path: "/x".into() }, wait)
                })
            })
            .collect();
        while s.1.asks.lock().waiting.len() < ASKS_PER_APP {
            std::thread::yield_now();
        }
        let more = s.1.asks.ask_from(&s.0, &greedy, Ask::ClipboardRead, wait);
        assert_eq!(
            more.message.as_deref(),
            Some(BUSY),
            "the app's own share is full"
        );
        // The same app in another process is the same app.
        let twin = Caller {
            pid: 11,
            ..greedy.clone()
        };
        let more = s.1.asks.ask_from(&s.0, &twin, Ask::ClipboardRead, wait);
        assert_eq!(more.message.as_deref(), Some(BUSY));
        let other = Caller {
            uid: 1000,
            pid: 12,
            app: "/usr/bin/notes".into(),
        };
        let short = Duration::from_millis(50);
        let a = s.1.asks.ask_from(&s.0, &other, Ask::ClipboardRead, short);
        assert_eq!(
            a.message.as_deref(),
            Some(NO_ANSWER),
            "admitted, then unanswered"
        );
        const { assert!(ASKS_PER_APP < ASKS_PER_UID) };
        for w in parked {
            assert_eq!(w.join().unwrap().message.as_deref(), Some(NO_ANSWER));
        }
    }

    /// QA-100 API-6: web app hosts share one executable; the cap counts each app on its own.
    #[test]
    fn per_app_cap_tells_web_apps_apart() {
        let host = |pid| Caller {
            uid: 1000,
            pid,
            app: crate::webapp::HOST.into(),
        };
        let scope = |pid: i32| {
            let id = if pid < 100 { "doctor" } else { "notes" };
            Some(format!(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-wol-org.wolos.webapp.{id}-{pid}-1.scope\n"
            ))
        };
        let (doctor, doctor2, notes) = (host(50), host(51), host(200));
        assert_eq!(app_key(&doctor, &scope), "webapp:org.wolos.webapp.doctor");
        assert_ne!(app_key(&doctor, &scope), app_key(&notes, &scope));
        // Two processes of one app are one app only while they share a scope name (pid is part of it).
        assert!(app_key(&doctor2, &scope).starts_with("webapp:"));
        // No scope readable: the executable, as before.
        assert_eq!(app_key(&doctor, &|_| None), crate::webapp::HOST);
        let other = Caller {
            uid: 1000,
            pid: 9,
            app: "/usr/bin/notes".into(),
        };
        assert_eq!(app_key(&other, &scope), "/usr/bin/notes");
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

    struct Counting(Arc<std::sync::atomic::AtomicUsize>);
    struct Token(Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for Token {
        fn drop(&mut self) {
            self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    impl crate::facts::Inhibit for Counting {
        fn hold(&self, _: &str, _: &str) -> Option<Box<dyn Send>> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(Box::new(Token(self.0.clone())))
        }
    }

    #[test]
    fn keep_awake_needs_the_grant_and_is_bounded_per_account() {
        use std::sync::atomic::Ordering::SeqCst;
        let live: Arc<std::sync::atomic::AtomicUsize> = Arc::default();
        let st = State {
            inhibit: Box::new(Counting(live.clone())),
            ..State::default()
        };
        let bus = Bus::<UnixStream>::default();
        let g = Fixed(&[("/usr/bin/a", "keep-awake")]);
        let line = r#"{"call":"power.keep-awake","reason":"Exporting"}"#;
        let run = |who: &Caller| respond(line, Some(who), &g, &bus, &st, None);
        assert_eq!(
            run(&caller("/usr/bin/b")).0.code,
            Some(Code::PermissionDenied)
        );
        assert_eq!(live.load(SeqCst), 0);
        let a = caller("/usr/bin/a");
        let mut held: Vec<Held> = Vec::new();
        for _ in 0..shared::portal::MAX_HOLDS {
            let (ans, extra) = run(&a);
            assert!(ans.ok);
            held.push(extra.hold.unwrap());
        }
        assert_eq!(
            run(&a).0.code,
            Some(Code::Unavailable),
            "one account's limit"
        );
        let mut other = a.clone();
        other.uid = 1001;
        let (ok, extra) = run(&other);
        assert!(ok.ok, "another account has its own");
        st.holds.give(extra.hold.unwrap().uid);
        let gone = held.pop().unwrap();
        st.holds.give(gone.uid);
        drop(gone);
        assert_eq!(live.load(SeqCst), shared::portal::MAX_HOLDS - 1);
        assert!(run(&a).0.ok, "a released hold can be taken again");
    }

    #[test]
    fn network_facts_need_their_grant() {
        let st = State::default();
        let bus = Bus::<UnixStream>::default();
        let line = r#"{"call":"network.info"}"#;
        let g = Fixed(&[]);
        let a = handle(line, Some(&caller("/usr/bin/a")), &g, &bus, &st);
        assert_eq!(a.code, Some(Code::PermissionDenied));
    }

    #[test]
    fn reading_audio_does_not_allow_changing_it() {
        let s = Arc::new((Bus::<UnixStream>::default(), State::default()));
        let g = Fixed(&[("/usr/bin/a", "audio-info")]);
        let a = caller("/usr/bin/a");
        let set = r#"{"call":"audio.set-volume","percent":10}"#;
        assert_eq!(
            handle(set, Some(&a), &g, &s.0, &s.1).code,
            Some(Code::PermissionDenied)
        );
        let t = shell(s.clone(), 1000, |ask| {
            assert_eq!(ask, Ask::AudioInfo);
            r#"{"call":"session.answer","id":ID,"data":{"volume":40,"muted":false}}"#.into()
        });
        let got = handle(r#"{"call":"audio.info"}"#, Some(&a), &g, &s.0, &s.1);
        assert_eq!(got.data.unwrap()["volume"], 40);
        t.join().unwrap();
    }

    #[test]
    fn providing_search_needs_its_grant_and_ends_with_the_connection() {
        let st = State::default();
        let bus = Bus::<UnixStream>::default();
        let g = Fixed(&[("/usr/bin/a", "search-provider")]);
        let line = r#"{"call":"search.provide"}"#;
        let run = |who: &Caller| respond(line, Some(who), &g, &bus, &st, None);
        assert_eq!(
            run(&caller("/usr/bin/b")).0.code,
            Some(Code::PermissionDenied)
        );
        let a = caller("/usr/bin/a");
        let (ans, extra) = run(&a);
        assert!(ans.ok);
        assert!(st.searches.is_provider(a.uid, a.pid));
        drop(extra);
        assert!(!st.searches.is_provider(a.uid, a.pid));
    }

    #[test]
    fn only_searchbar_collects_or_opens_rows() {
        let st = State::default();
        let bus = Bus::<UnixStream>::default();
        let g = Fixed(&[("/usr/bin/a", "search-provider")]);
        for line in [
            r#"{"call":"search.collect","text":"secret"}"#,
            r#"{"call":"search.open","provider":1,"key":"k"}"#,
        ] {
            let a = handle(line, Some(&caller("/usr/bin/a")), &g, &bus, &st);
            assert_eq!(a.code, Some(Code::PermissionDenied), "{line}");
            let shell = handle(line, Some(&caller(SHOWERS[0])), &g, &bus, &st);
            assert_eq!(shell.code, Some(Code::PermissionDenied), "{line}");
        }
        let spot = caller(SEARCHBAR);
        let a = handle(
            r#"{"call":"search.collect","text":"q"}"#,
            Some(&spot),
            &g,
            &bus,
            &st,
        );
        assert_eq!(a.data, Some(serde_json::json!({"results": []})));
    }

    #[test]
    fn a_message_needs_a_grant_for_that_peer() {
        let st = State::default();
        let bus = Bus::<UnixStream>::default();
        let g = Fixed(&[
            ("/usr/bin/notes", "ipc"),
            ("/usr/bin/mail", "ipc:/usr/bin/notes"),
        ]);
        let (notes, mail) = (caller("/usr/bin/notes"), caller("/usr/bin/mail"));
        let listen = r#"{"call":"ipc.listen","name":"greet"}"#;
        let (ans, extra) = respond(listen, Some(&notes), &g, &bus, &st, None);
        assert!(ans.ok);
        let denied = respond(listen, Some(&mail), &g, &bus, &st, None).0;
        assert_eq!(
            denied.code,
            Some(Code::PermissionDenied),
            "listening needs ipc"
        );
        let send = |to: &str, who: &Caller| {
            let line = format!(r#"{{"call":"ipc.send","peer":"{to}","name":"greet","body":"x"}}"#);
            handle(&line, Some(who), &g, &bus, &st)
        };
        let other = send("/usr/bin/shell", &mail);
        assert_eq!(other.code, Some(Code::PermissionDenied), "another peer");
        assert!(other.message.unwrap().contains("not allowed"));
        let reverse = send("/usr/bin/mail", &notes);
        assert_eq!(reverse.code, Some(Code::PermissionDenied), "one way");
        drop(extra);
        let nobody = send("/usr/bin/notes", &mail);
        assert_eq!(nobody.code, Some(Code::Unavailable));
    }

    #[test]
    fn opening_a_device_needs_a_grant_for_that_device() {
        let st = State::default();
        let bus = Bus::<UnixStream>::default();
        let g = Fixed(&[
            ("/usr/bin/a", "device:1234:5678"),
            ("/usr/bin/b", "devices"),
        ]);
        let open = |who: &str, vendor: u32| {
            let line = format!(r#"{{"call":"device.open","vendor":{vendor},"product":22136}}"#);
            handle(&line, Some(&caller(who)), &g, &bus, &st).code
        };
        assert_eq!(
            open("/usr/bin/a", 0x1234),
            Some(Code::Unavailable),
            "allowed: gone"
        );
        assert_eq!(open("/usr/bin/a", 0x1235), Some(Code::PermissionDenied));
        assert_eq!(
            open("/usr/bin/b", 0x1234),
            Some(Code::PermissionDenied),
            "listing only"
        );
    }

    #[test]
    fn a_service_needs_its_grant() {
        let st = State::default();
        let bus = Bus::<UnixStream>::default();
        let g = Fixed(&[("/usr/bin/a", "background")]);
        let line = r#"{"call":"service.start","name":"sync"}"#;
        let no = handle(line, Some(&caller("/usr/bin/b")), &g, &bus, &st);
        assert_eq!(no.code, Some(Code::PermissionDenied));
        let yes = handle(line, Some(&caller("/usr/bin/a")), &g, &bus, &st);
        assert_eq!(
            yes.code,
            Some(Code::Unavailable),
            "none in the default state"
        );
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

    /// MANUAL 0.12.0: hello's notification was refused and Settings had no row for hello. The
    /// refusal still refuses; wold also notes (app, capability) once in the account's own file.
    #[test]
    fn a_refused_app_is_noted_for_settings() {
        let dir = std::env::temp_dir().join(format!("wol-portal-asked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let g = Remembering {
            grants: Fixed(&[("/usr/bin/good", "notifications")]),
            dir: dir.clone(),
            log: |m| panic!("{m}"),
        };
        let uid = shared::sys::euid();
        let who = Caller {
            uid,
            pid: 1,
            app: "/home/a/Work/hello/hello".into(),
        };
        let bus = Bus::<UnixStream>::default();
        for _ in 0..3 {
            let a = handle(POST, Some(&who), &g, &bus, &State::default());
            assert_eq!(a.code, Some(Code::PermissionDenied));
        }
        // Granted (no desktop listening here, so Unavailable): not a refusal, nothing noted.
        let ok = handle(
            POST,
            Some(&caller("/usr/bin/good")),
            &g,
            &bus,
            &State::default(),
        );
        assert_ne!(ok.code, Some(Code::PermissionDenied));
        let f = dir.join(format!("{uid}.json"));
        let text = std::fs::read_to_string(&f).unwrap();
        assert_eq!(
            wol_core::consent::asked(&text),
            [("/home/a/Work/hello/hello".into(), "notifications".into())],
            "once, and never the granted app: {text}"
        );
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let m = std::fs::metadata(&f).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o600);
        assert_eq!(m.uid(), uid);
        // A huge file the account wrote is read only so far, and replaced whole.
        std::fs::write(&f, "x".repeat(200_000)).unwrap();
        handle(POST, Some(&who), &g, &bus, &State::default());
        assert_eq!(
            wol_core::consent::asked(&std::fs::read_to_string(&f).unwrap()).len(),
            1
        );
        let _ = std::fs::remove_dir_all(dir);
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
