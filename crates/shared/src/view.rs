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
/// Image sizes in px.
pub const IMAGE_SIZES: [u16; 7] = [16, 24, 32, 48, 64, 96, 128];
const IMAGE_FILES: [&str; 4] = [".png", ".svg", ".jpg", ".jpeg"];

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

/// Interactive parts carry an `id`, unique in their tree; what the person does comes back as a
/// [`FromHost`] event with that id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// A click comes back as [`FromHost::Clicked`].
    Button {
        id: u32,
        label: String,
    },
    Spacer,
    /// One line of text; each edit comes back as [`FromHost::Edited`].
    TextField {
        id: u32,
        #[serde(default)]
        text: String,
        #[serde(default)]
        placeholder: String,
    },
    /// A switch with an optional label; [`FromHost::Toggled`].
    Toggle {
        id: u32,
        on: bool,
        #[serde(default)]
        label: String,
    },
    /// [`FromHost::Slid`].
    Slider {
        id: u32,
        value: f64,
        min: f64,
        max: f64,
    },
    /// A symbolic icon name, or an absolute path to a PNG, SVG or JPEG.
    Image {
        source: String,
        size: u16,
    },
    /// Rows with separators (Kit list).
    List {
        children: Vec<Node>,
    },
}

/// Icon name, or a clean absolute image path.
pub fn image_source_ok(s: &str) -> bool {
    if let Some(rest) = s.strip_prefix('/') {
        let lower = s.to_ascii_lowercase();
        s.len() <= 1024
            && rest
                .split('/')
                .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
            && !s.chars().any(char::is_control)
            && IMAGE_FILES.iter().any(|e| lower.ends_with(e))
    } else {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    }
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
        let mut unique = |id: &u32| {
            if ids.contains(id) {
                return Err(format!("Part id {id} is used twice."));
            }
            ids.push(*id);
            Ok(())
        };
        match self {
            Node::Stack { spacing, children } | Node::Row { spacing, children } => {
                if !SPACING.contains(spacing) {
                    return Err(format!("Spacing is one of {SPACING:?}."));
                }
                children
                    .iter()
                    .try_for_each(|c| c.check(depth + 1, count, ids))
            }
            Node::List { children } => children
                .iter()
                .try_for_each(|c| c.check(depth + 1, count, ids)),
            Node::TextField {
                id,
                text,
                placeholder,
            } => {
                if !text_ok(text) || !text_ok(placeholder) || text.contains('\n') {
                    return Err(format!(
                        "A text field holds one line of at most {MAX_TEXT} bytes."
                    ));
                }
                unique(id)
            }
            Node::Toggle { id, label, .. } => {
                if !text_ok(label) {
                    return Err("A toggle's label is short text.".into());
                }
                unique(id)
            }
            Node::Slider {
                id,
                value,
                min,
                max,
            } => {
                if !(min.is_finite() && max.is_finite() && value.is_finite())
                    || min >= max
                    || value < min
                    || value > max
                {
                    return Err("A slider needs min < max and a value between them.".into());
                }
                unique(id)
            }
            Node::Image { source, size } => {
                if !IMAGE_SIZES.contains(size) {
                    return Err(format!("Image size is one of {IMAGE_SIZES:?}."));
                }
                if !image_source_ok(source) {
                    return Err(
                        "An image is an icon name or an absolute path to a PNG, SVG or JPEG file."
                            .into(),
                    );
                }
                Ok(())
            }
            Node::Label { text, .. } if !text_ok(text) => {
                Err(format!("A label is text of at most {MAX_TEXT} bytes."))
            }
            Node::Button { id, label } => {
                if label.trim().is_empty() || !text_ok(label) {
                    return Err("A button needs a short label.".into());
                }
                unique(id)
            }
            Node::Label { .. } | Node::Spacer => Ok(()),
        }
    }
}

/// App -> Kit host. The first message is [`ToHost::Hello`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum ToHost {
    /// The app's reverse-DNS id (window class; not an identity check).
    Hello {
        app: String,
    },
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum FromHost {
    Clicked {
        window: u32,
        widget: u32,
    },
    /// The person closed the window.
    Closed {
        window: u32,
    },
    Edited {
        window: u32,
        widget: u32,
        text: String,
    },
    Toggled {
        window: u32,
        widget: u32,
        on: bool,
    },
    Slid {
        window: u32,
        widget: u32,
        value: f64,
    },
}

/// Reverse-DNS app id: `org.example.hello`.
pub fn is_app_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.contains('.')
        && !s.starts_with('.')
        && !s.ends_with('.')
        && !s.contains("..")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
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
            Self::Hello { app } => is_app_id(app).then_some(m),
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
        let e: Self = serde_json::from_str(line.trim()).ok()?;
        match &e {
            Self::Slid { value, .. } if !value.is_finite() => None,
            Self::Edited { text, .. } if text.len() > MAX_TEXT => None,
            _ => Some(e),
        }
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

    #[test]
    fn one_one_parts_validate() {
        let tf = |id| Node::TextField {
            id,
            text: String::new(),
            placeholder: "Name".into(),
        };
        let slider = |min, value, max| Node::Slider {
            id: 9,
            value,
            min,
            max,
        };
        let image = |s: &str, size| Node::Image {
            source: s.into(),
            size,
        };
        let cases: Vec<(Node, bool)> = vec![
            (
                Node::Row {
                    spacing: 8,
                    children: vec![
                        tf(1),
                        Node::Spacer,
                        Node::Toggle {
                            id: 2,
                            on: true,
                            label: "Wi-Fi".into(),
                        },
                    ],
                },
                true,
            ),
            (
                Node::List {
                    children: vec![tf(1), tf(1)],
                },
                false,
            ),
            (slider(0.0, 0.5, 1.0), true),
            (slider(1.0, 0.5, 0.0), false),
            (slider(0.0, 2.0, 1.0), false),
            (slider(0.0, f64::NAN, 1.0), false),
            (image("weather-clear-symbolic", 32), true),
            (image("/usr/share/wolos/a.png", 64), true),
            (image("/usr/share/../etc/shadow.png", 64), false),
            (image("/home/x/a.txt", 64), false),
            (image("a b", 32), false),
            (image("x", 33), false),
            (
                Node::Stack {
                    spacing: 0,
                    children: vec![
                        Node::Button {
                            id: 4,
                            label: "x".into(),
                        },
                        tf(4),
                    ],
                },
                false,
            ),
        ];
        for (n, ok) in cases {
            assert_eq!(n.validate().is_ok(), ok, "{n:?}");
        }
    }

    #[test]
    fn one_one_wire() {
        let h = ToHost::Hello {
            app: "org.example.hello".into(),
        };
        assert_eq!(
            h.to_line(),
            "{\"op\":\"hello\",\"app\":\"org.example.hello\"}\n"
        );
        assert_eq!(ToHost::parse(&h.to_line()), Some(h));
        assert_eq!(ToHost::parse(r#"{"op":"hello","app":"no id"}"#), None);
        for e in [
            FromHost::Edited {
                window: 1,
                widget: 2,
                text: "hi".into(),
            },
            FromHost::Toggled {
                window: 1,
                widget: 3,
                on: true,
            },
            FromHost::Slid {
                window: 1,
                widget: 4,
                value: 0.25,
            },
            FromHost::Closed { window: 1 },
        ] {
            assert_eq!(FromHost::parse(&e.to_line()), Some(e));
        }
        assert_eq!(
            FromHost::parse(r#"{"event":"slid","window":1,"widget":4,"value":1e999}"#),
            None
        );
    }
}
