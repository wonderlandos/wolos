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
}
