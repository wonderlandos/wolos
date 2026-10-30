//! Kit view trees (ADR 0020): what an app window or a Lua widget shows, as data. libwol and the Lua
//! runtime build them; the Kit host (the shell) draws them with Kit parts and sends clicks back.
//! Wire: one JSON object per line, [`ToHost`] from the app, [`FromHost`] from the host.
use serde::{Deserialize, Serialize};

/// Spacing scale (HIG §2), same as desktop widget layouts.
pub const SPACING: [u8; 6] = [0, 4, 8, 12, 16, 24];
pub const MAX_DEPTH: usize = 12;
pub const MAX_NODES: usize = 200;
pub const MAX_TEXT: usize = 1024;
pub const MAX_TITLE: usize = 256;

/// Text styles of the Kit (`shared::widgets` roles).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Display,
    Large,
    Title,
    #[default]
    Body,
    Secondary,
    Caption,
}

impl Role {
    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.into())).ok()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Node {
    Stack {
        #[serde(default)]
        spacing: u8,
        children: Vec<Node>,
    },
    Row {
        #[serde(default)]
        spacing: u8,
        children: Vec<Node>,
    },
    Label {
        text: String,
        #[serde(default)]
        role: Role,
    },
    /// `id` is unique in its tree; a click comes back as [`FromHost::Clicked`].
    Button {
        id: u32,
        label: String,
    },
    Spacer,
}

impl Node {
    /// Depth, node count, spacing scale, text length, unique button ids.
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = Vec::new();
        let mut count = 0;
        self.check(1, &mut count, &mut ids)
    }

    fn check(&self, depth: usize, count: &mut usize, ids: &mut Vec<u32>) -> Result<(), String> {
        *count += 1;
        if depth > MAX_DEPTH {
            return Err(format!("A view is at most {MAX_DEPTH} levels deep."));
        }
        if *count > MAX_NODES {
            return Err(format!("A view has at most {MAX_NODES} parts."));
        }
        let text_ok =
            |t: &str| t.len() <= MAX_TEXT && !t.chars().any(|c| c.is_control() && c != '\n');
        match self {
            Node::Stack { spacing, children } | Node::Row { spacing, children } => {
                if !SPACING.contains(spacing) {
                    return Err(format!("Spacing is one of {SPACING:?}."));
                }
                children
                    .iter()
                    .try_for_each(|c| c.check(depth + 1, count, ids))
            }
            Node::Label { text, .. } if !text_ok(text) => {
                Err(format!("A label is text of at most {MAX_TEXT} bytes."))
            }
            Node::Button { id, label } => {
                if label.trim().is_empty() || !text_ok(label) {
                    return Err("A button needs a short label.".into());
                }
                if ids.contains(id) {
                    return Err(format!("Button id {id} is used twice."));
                }
                ids.push(*id);
                Ok(())
            }
            Node::Label { .. } | Node::Spacer => Ok(()),
        }
    }
}

/// App -> Kit host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum ToHost {
    /// Show or replace a window's content.
    Show {
        window: u32,
        title: String,
        content: Node,
    },
    Close {
        window: u32,
    },
}

/// Kit host -> app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum FromHost {
    Clicked { window: u32, widget: u32 },
    Closed { window: u32 },
}

impl ToHost {
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }

    /// Valid message or None (the host drops it).
    pub fn parse(line: &str) -> Option<Self> {
        let m: Self = serde_json::from_str(line.trim()).ok()?;
        match &m {
            Self::Show { title, content, .. } => (title.len() <= MAX_TITLE
                && !title.chars().any(char::is_control)
                && content.validate().is_ok())
            .then_some(m),
            Self::Close { .. } => Some(m),
        }
    }
}

impl FromHost {
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

    fn label(t: &str) -> Node {
        Node::Label {
            text: t.into(),
            role: Role::Body,
        }
    }

    #[test]
    fn json_shape() {
        let n = Node::Stack {
            spacing: 8,
            children: vec![
                label("Hi"),
                Node::Button {
                    id: 1,
                    label: "OK".into(),
                },
                Node::Spacer,
            ],
        };
        let j = serde_json::to_string(&n).unwrap();
        assert_eq!(
            j,
            r#"{"type":"stack","spacing":8,"children":[{"type":"label","text":"Hi","role":"body"},{"type":"button","id":1,"label":"OK"},{"type":"spacer"}]}"#
        );
        assert_eq!(serde_json::from_str::<Node>(&j).unwrap(), n);
        assert_eq!(Role::parse("secondary"), Some(Role::Secondary));
        assert_eq!(Role::parse("huge"), None);
    }

    #[test]
    fn validate_table() {
        let b = |id| Node::Button {
            id,
            label: "x".into(),
        };
        let stack = |spacing, children| Node::Stack { spacing, children };
        let mut deep = label("x");
        for _ in 0..MAX_DEPTH {
            deep = stack(0, vec![deep]);
        }
        let cases: Vec<(Node, bool)> = vec![
            (stack(8, vec![b(1), b(2)]), true),
            (stack(8, vec![b(1), b(1)]), false),
            (stack(5, vec![]), false),
            (label(&"x".repeat(MAX_TEXT + 1)), false),
            (label("two\nlines"), true),
            (label("bell\u{7}"), false),
            (
                Node::Button {
                    id: 1,
                    label: " ".into(),
                },
                false,
            ),
            (stack(0, (0..MAX_NODES as u32).map(b).collect()), false),
            (deep, false),
        ];
        for (n, ok) in cases {
            assert_eq!(n.validate().is_ok(), ok, "{n:?}");
        }
    }

    #[test]
    fn host_wire() {
        let show = ToHost::Show {
            window: 1,
            title: "Hello".into(),
            content: label("Hi"),
        };
        assert_eq!(ToHost::parse(&show.to_line()), Some(show));
        let bad = r#"{"op":"show","window":1,"title":"x","content":{"type":"stack","spacing":3,"children":[]}}"#;
        assert_eq!(ToHost::parse(bad), None);
        let c = FromHost::Clicked {
            window: 1,
            widget: 7,
        };
        assert_eq!(
            c.to_line(),
            "{\"event\":\"clicked\",\"window\":1,\"widget\":7}\n"
        );
        assert_eq!(FromHost::parse(&c.to_line()), Some(c));
        assert_eq!(FromHost::parse("{\"event\":\"nope\"}"), None);
    }
}
