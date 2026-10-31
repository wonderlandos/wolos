//! Event bus wire: `subscribe` on the wold socket, then one JSON object per line.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    /// Frontmost window changed (`class` = app id). pid 0 = unknown.
    ActiveWindow {
        class: String,
        title: String,
        pid: i32,
    },
    OpenWindow {
        address: String,
        workspace: String,
        class: String,
        title: String,
    },
    CloseWindow {
        address: String,
    },
    Workspace {
        name: String,
    },
    Theme {
        id: String,
    },
    Job {
        label: String,
        state: String,
    },
    /// A libwol app's notification (ADR 0020), made by wold after its permission check. Reaches only
    /// the notification shower of account `uid`; `app` is the caller's executable as the kernel saw
    /// it. `id` is wold's; a pressed button goes back as [`Event::NotificationAction`].
    Notification {
        uid: u32,
        app: String,
        title: String,
        body: String,
        #[serde(default)]
        id: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        actions: Vec<crate::portal::NoteAction>,
        /// New text for notification `id`, which is already showing.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        replace: bool,
    },
    /// The app took notification `id` back: same audience as [`Event::Notification`].
    /// wold asks the desktop of account `uid` to do something a checked call needs. `id` goes back
    /// in `portal::Call::SessionAnswer`.
    SessionAsk {
        uid: u32,
        id: u32,
        ask: crate::portal::Ask,
    },
    NotificationWithdrawn {
        uid: u32,
        id: u32,
    },
    /// A search for provider process `pid`; `id` goes back in `portal::Call::SearchReply`.
    SearchQuery {
        uid: u32,
        pid: i32,
        id: u32,
        text: String,
    },
    /// The person opened row `key` of provider process `pid`.
    SearchOpen {
        uid: u32,
        pid: i32,
        key: String,
    },
    /// A message from another app of the account, for process `pid`. `from` is the sender's
    /// executable as the kernel saw it; `id` 0 was published and takes no reply.
    Message {
        uid: u32,
        pid: i32,
        id: u32,
        from: String,
        name: String,
        body: String,
    },
    /// A notification button pressed: reaches only process `pid` (the one that posted `id`).
    NotificationAction {
        uid: u32,
        pid: i32,
        id: u32,
        key: String,
    },
}

/// Who may receive an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Audience {
    Everyone,
    /// The notification shower of this account.
    Account(u32),
    /// The subscriber connection of this process (and account).
    Process {
        uid: u32,
        pid: i32,
    },
}

impl Event {
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }

    pub fn parse(line: &str) -> Option<Self> {
        serde_json::from_str(line.trim()).ok()
    }

    pub fn audience(&self) -> Audience {
        match self {
            Self::Notification { uid, .. }
            | Self::NotificationWithdrawn { uid, .. }
            | Self::SessionAsk { uid, .. } => Audience::Account(*uid),
            Self::NotificationAction { uid, pid, .. }
            | Self::SearchQuery { uid, pid, .. }
            | Self::SearchOpen { uid, pid, .. }
            | Self::Message { uid, pid, .. } => Audience::Process {
                uid: *uid,
                pid: *pid,
            },
            _ => Audience::Everyone,
        }
    }

    /// `publish` from a client: only window and session events; wold makes the rest itself.
    pub fn client_may_publish(&self) -> bool {
        self.audience() == Audience::Everyone && !matches!(self, Self::Job { .. })
    }

    /// Wire name (`event` field): the topic libwol subscribes to.
    pub fn topic(&self) -> &'static str {
        match self {
            Self::ActiveWindow { .. } => "active-window",
            Self::OpenWindow { .. } => "open-window",
            Self::CloseWindow { .. } => "close-window",
            Self::Workspace { .. } => "workspace",
            Self::Theme { .. } => "theme",
            Self::Job { .. } => "job",
            Self::Notification { .. } => "notification",
            Self::NotificationWithdrawn { .. } => "notification-withdrawn",
            Self::SearchQuery { .. } => "search-query",
            Self::SearchOpen { .. } => "search-open",
            Self::Message { .. } => "message",
            Self::SessionAsk { .. } => "session-ask",
            Self::NotificationAction { .. } => "notification-action",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format() {
        let e = Event::ActiveWindow {
            class: "foot".into(),
            title: "~".into(),
            pid: 7,
        };
        assert_eq!(
            e.to_line(),
            "{\"event\":\"active-window\",\"class\":\"foot\",\"title\":\"~\",\"pid\":7}\n"
        );
        assert_eq!(Event::parse(&e.to_line()), Some(e));
        assert_eq!(Event::parse("{\"event\":\"nope\"}"), None);
    }

    #[test]
    fn a_search_reaches_one_process_and_cannot_be_forged() {
        let (uid, pid) = (1000, 7);
        for e in [
            Event::SearchQuery {
                uid,
                pid,
                id: 1,
                text: "q".into(),
            },
            Event::SearchOpen {
                uid,
                pid,
                key: "k".into(),
            },
            Event::Message {
                uid,
                pid,
                id: 1,
                from: "/usr/bin/a".into(),
                name: "greet".into(),
                body: "hi".into(),
            },
        ] {
            assert_eq!(e.audience(), Audience::Process { uid, pid });
            assert!(!e.client_may_publish());
            assert_eq!(Event::parse(&e.to_line()), Some(e));
        }
    }

    #[test]
    fn notification_is_scoped_and_not_forgeable() {
        let n = Event::Notification {
            uid: 1000,
            app: "/usr/bin/foo".into(),
            title: "t".into(),
            body: String::new(),
            id: 1,
            actions: Vec::new(),
            replace: false,
        };
        assert_eq!(n.audience(), Audience::Account(1000));
        assert!(!n.client_may_publish());
        assert!(
            !n.to_line().contains("replace"),
            "1.1 wire unchanged for a first post"
        );
        let ask = Event::SessionAsk {
            uid: 1000,
            id: 1,
            ask: crate::portal::Ask::ClipboardRead,
        };
        assert_eq!(ask.audience(), Audience::Account(1000));
        assert!(!ask.client_may_publish(), "apps cannot forge an ask");
        assert_eq!(Event::parse(&ask.to_line()), Some(ask));
        let gone = Event::NotificationWithdrawn { uid: 1000, id: 1 };
        assert_eq!(gone.audience(), Audience::Account(1000));
        assert!(!gone.client_may_publish());
        let a = Event::NotificationAction {
            uid: 1000,
            pid: 42,
            id: 1,
            key: "ok".into(),
        };
        assert_eq!(a.audience(), Audience::Process { uid: 1000, pid: 42 });
        assert!(!a.client_may_publish());
        let w = Event::Workspace { name: "1".into() };
        assert_eq!(
            (w.audience(), w.client_may_publish()),
            (Audience::Everyone, true)
        );
        for e in [n, a, w, gone] {
            let line = e.to_line();
            assert!(
                line.contains(&format!("\"event\":\"{}\"", e.topic())),
                "{line}"
            );
        }
    }
}
