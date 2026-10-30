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
    /// subscribers of account `uid` (and root); `app` is the caller's executable as the kernel saw it.
    Notification {
        uid: u32,
        app: String,
        title: String,
        body: String,
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

    /// Account that may receive it; None = every subscriber.
    pub fn audience(&self) -> Option<u32> {
        match self {
            Self::Notification { uid, .. } => Some(*uid),
            _ => None,
        }
    }

    /// `publish` from a client: only window and session events; wold makes the rest itself.
    pub fn client_may_publish(&self) -> bool {
        !matches!(self, Self::Notification { .. } | Self::Job { .. })
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
    fn notification_is_scoped_and_not_forgeable() {
        let n = Event::Notification {
            uid: 1000,
            app: "/usr/bin/foo".into(),
            title: "t".into(),
            body: String::new(),
        };
        assert_eq!(n.audience(), Some(1000));
        assert!(!n.client_may_publish());
        let w = Event::Workspace { name: "1".into() };
        assert_eq!((w.audience(), w.client_may_publish()), (None, true));
        for e in [n, w] {
            let line = e.to_line();
            assert!(
                line.contains(&format!("\"event\":\"{}\"", e.topic())),
                "{line}"
            );
        }
    }
}
