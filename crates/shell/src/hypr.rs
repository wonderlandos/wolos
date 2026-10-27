//! Hyprland IPC: `.socket2.sock` events (event-driven, no polling) and `j/clients` on `.socket.sock`.
//! Replace with the `realtime` crate's client when it lands.
use std::io::{Read, Write};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    ActiveWindow { class: String, title: String },
    OpenWindow { address: String, class: String },
    CloseWindow { address: String },
    Workspace(String),
    Other,
}

/// One `EVENT>>DATA` line.
pub fn parse_event(line: &str) -> Option<Event> {
    let (name, data) = line.trim_end().split_once(">>")?;
    Some(match name {
        "activewindow" => {
            let (class, title) = data.split_once(',').unwrap_or((data, ""));
            Event::ActiveWindow {
                class: class.into(),
                title: title.into(),
            }
        }
        "openwindow" => {
            let mut p = data.splitn(4, ',');
            let address = p.next()?.into();
            let _ws = p.next()?;
            Event::OpenWindow {
                address,
                class: p.next()?.into(),
            }
        }
        "closewindow" => Event::CloseWindow {
            address: data.into(),
        },
        "workspace" => Event::Workspace(data.into()),
        _ => Event::Other,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct Client {
    pub address: String,
    pub class: String,
    pub title: String,
    /// 0 = most recently focused.
    #[serde(rename = "focusHistoryID", default = "far")]
    pub focus: i64,
    #[serde(default)]
    pub workspace: WorkspaceRef,
    #[serde(default)]
    pub at: [i32; 2],
    #[serde(default)]
    pub size: [i32; 2],
    #[serde(default)]
    pub tags: Vec<String>,
}

fn far() -> i64 {
    i64::MAX
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WorkspaceRef {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
}

impl Client {
    pub fn new(address: &str, class: &str, title: &str) -> Self {
        Client {
            address: address.into(),
            class: class.into(),
            title: title.into(),
            focus: far(),
            workspace: WorkspaceRef::default(),
            at: [0, 0],
            size: [0, 0],
            tags: Vec::new(),
        }
    }
}

pub fn parse_clients(json: &str) -> Vec<Client> {
    serde_json::from_str(json).unwrap_or_default()
}

/// `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/<name>`; None outside Hyprland.
pub fn socket(name: &str) -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let rt = std::env::var("XDG_RUNTIME_DIR").ok()?;
    Some(PathBuf::from(rt).join("hypr").join(sig).join(name))
}

/// One request on `.socket.sock`.
pub fn request(cmd: &str) -> Option<String> {
    let mut s = std::os::unix::net::UnixStream::connect(socket(".socket.sock")?).ok()?;
    s.write_all(cmd.as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

pub fn clients() -> Vec<Client> {
    request("j/clients")
        .map(|j| parse_clients(&j))
        .unwrap_or_default()
}

/// `j/activewindow` -> (class, title); ("", "") on the empty desktop (`{}`), None if unreadable.
pub fn parse_active(json: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    Some((s("class"), s("title")))
}

/// `j/activewindow` as a client (address, class, size, tags); None on the empty desktop.
pub fn parse_active_client(json: &str) -> Option<Client> {
    serde_json::from_str::<Client>(json)
        .ok()
        .filter(|c| !c.address.is_empty())
}

/// `j/layers` -> namespaces of every mapped layer surface, all outputs and levels.
pub fn parse_layers(json: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let mut out = Vec::new();
    for mon in v.as_object().into_iter().flat_map(|m| m.values()) {
        for level in mon["levels"]
            .as_object()
            .into_iter()
            .flat_map(|l| l.values())
        {
            for l in level.as_array().into_iter().flatten() {
                if let Some(n) = l["namespace"].as_str() {
                    out.push(n.to_string());
                }
            }
        }
    }
    out
}

/// `j/monitors`: the focused output's logical box minus reserved areas (menu bar, dock).
pub fn parse_zone(json: &str) -> Option<(i32, i32, i32, i32)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let m = v
        .as_array()?
        .iter()
        .find(|m| m["focused"].as_bool() == Some(true))?;
    let n = |k: &str| m[k].as_f64().unwrap_or_default();
    let r = |i: usize| m["reserved"][i].as_f64().unwrap_or_default();
    let scale = if n("scale") > 0.0 { n("scale") } else { 1.0 };
    let (w, h) = (n("width") / scale, n("height") / scale);
    let i = |f: f64| f.round() as i32;
    Some((
        i(n("x") + r(0)),
        i(n("y") + r(1)),
        i(w - r(0) - r(2)),
        i(h - r(1) - r(3)),
    ))
}

/// Running classes, lowercase, for the dock's running dot.
pub fn running_classes(clients: &[Client]) -> std::collections::HashSet<String> {
    clients.iter().map(|c| c.class.to_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_is_the_focused_output_minus_reserved() {
        let j = r#"[{"x":0,"y":0,"width":2560,"height":1600,"scale":2.0,"focused":false,"reserved":[0,0,0,0]},
                    {"x":1280,"y":0,"width":2560,"height":1600,"scale":2.0,"focused":true,"reserved":[0,28,0,102]}]"#;
        assert_eq!(parse_zone(j), Some((1280, 28, 1280, 670)));
        assert_eq!(parse_zone("[]"), None);
        let a = parse_active_client(
            r#"{"address":"0xa","class":"foot","title":"t","size":[4,3],"tags":["wol-csd"]}"#,
        )
        .unwrap();
        assert_eq!((a.size, a.tags), ([4, 3], vec!["wol-csd".to_string()]));
        assert_eq!(parse_active_client("{}"), None);
    }

    #[test]
    fn parses_events() {
        assert_eq!(
            parse_event("activewindow>>foot,~/src, ok\n"),
            Some(Event::ActiveWindow {
                class: "foot".into(),
                title: "~/src, ok".into()
            })
        );
        assert_eq!(
            parse_event("openwindow>>55a1,1,firefox,Mozilla"),
            Some(Event::OpenWindow {
                address: "55a1".into(),
                class: "firefox".into()
            })
        );
        assert_eq!(
            parse_event("closewindow>>55a1"),
            Some(Event::CloseWindow {
                address: "55a1".into()
            })
        );
        assert_eq!(
            parse_event("activewindow>>,"),
            Some(Event::ActiveWindow {
                class: "".into(),
                title: "".into()
            })
        );
        assert_eq!(parse_event("garbage"), None);
    }

    /// Live: a restarted shell showed "Files" while a window was focused (no event until a change).
    #[test]
    fn parses_active_window() {
        for (json, want) in [
            (r#"{"class":"foot","title":"~"}"#, Some(("foot", "~"))),
            ("{}", Some(("", ""))),
            ("Invalid", None),
        ] {
            let got = parse_active(json);
            assert_eq!(
                got.as_ref().map(|(c, t)| (c.as_str(), t.as_str())),
                want,
                "{json}"
            );
        }
    }

    #[test]
    fn parses_clients_json() {
        let c = parse_clients(r#"[{"address":"0x1","class":"Foot","title":"t","pid":3}]"#);
        assert_eq!(c[0].class, "Foot");
        assert!(running_classes(&c).contains("foot"));
        assert!(parse_clients("not json").is_empty());
        let c = parse_clients(
            r#"[{"address":"0x2","class":"a","title":"","focusHistoryID":1,"workspace":{"id":2,"name":"2"},"at":[10,20],"size":[300,200]}]"#,
        );
        assert_eq!(
            (c[0].focus, c[0].workspace.id, c[0].at, c[0].size),
            (1, 2, [10, 20], [300, 200])
        );
    }
}
