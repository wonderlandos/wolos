//! Hyprland IPC: `.socket.sock` (requests, e.g. `j/activewindow`) and `.socket2.sock`
//! (events, one `name>>data` line each). Pure parsers + thin socket helpers.
use shared::events::Event;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/<name>`.
pub fn socket_path(runtime_dir: &str, signature: &str, name: &str) -> PathBuf {
    PathBuf::from(runtime_dir)
        .join("hypr")
        .join(signature)
        .join(name)
}

fn env_socket(name: &str) -> Option<PathBuf> {
    let rt = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    Some(socket_path(&rt, &sig, name))
}

/// One parsed `.socket2.sock` line. `ActiveAddress` comes from `activewindowv2` (pid looked up later).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HyprEvent {
    Bus(Event),
    ActiveAddress(String),
}

/// Parse one event line. Unknown events = None.
pub fn parse_event(line: &str) -> Option<HyprEvent> {
    let (name, data) = line.trim_end_matches(['\r', '\n']).split_once(">>")?;
    let ev = match name {
        "activewindow" => {
            let (class, title) = data.split_once(',')?;
            Event::ActiveWindow {
                class: class.into(),
                title: title.into(),
                pid: 0,
            }
        }
        "activewindowv2" => {
            return (!data.is_empty() && data != ",")
                .then(|| HyprEvent::ActiveAddress(data.into()));
        }
        "openwindow" => {
            let mut f = data.splitn(4, ',');
            Event::OpenWindow {
                address: f.next()?.into(),
                workspace: f.next()?.into(),
                class: f.next()?.into(),
                title: f.next()?.into(),
            }
        }
        "closewindow" => Event::CloseWindow {
            address: data.into(),
        },
        "workspace" => Event::Workspace { name: data.into() },
        _ => return None,
    };
    Some(HyprEvent::Bus(ev))
}

/// pid from a `j/activewindow` reply (`{"pid": 123, ...}`); None for `{}` or pid <= 0.
pub fn active_pid(json: &str) -> Option<i32> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let pid = i32::try_from(v.get("pid")?.as_i64()?).ok()?;
    (pid > 0).then_some(pid)
}

/// One request on `.socket.sock`; returns the whole reply.
pub fn request(cmd: &str) -> std::io::Result<String> {
    let path = env_socket(".socket.sock").ok_or(std::io::ErrorKind::NotFound)?;
    let mut s = UnixStream::connect(path)?;
    s.write_all(cmd.as_bytes())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    Ok(out)
}

/// Blocking iterator over `.socket2.sock` events.
pub fn events() -> std::io::Result<impl Iterator<Item = HyprEvent>> {
    let path = env_socket(".socket2.sock").ok_or(std::io::ErrorKind::NotFound)?;
    let r = BufReader::new(UnixStream::connect(path)?);
    Ok(r.lines()
        .map_while(Result::ok)
        .filter_map(|l| parse_event(&l)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_layout() {
        assert_eq!(
            socket_path("/run/user/1000", "abc", ".socket2.sock"),
            PathBuf::from("/run/user/1000/hypr/abc/.socket2.sock")
        );
    }

    #[test]
    fn event_table() {
        let bus = |e| Some(HyprEvent::Bus(e));
        let cases = [
            (
                "activewindow>>foot,~/src",
                bus(Event::ActiveWindow {
                    class: "foot".into(),
                    title: "~/src".into(),
                    pid: 0,
                }),
            ),
            (
                "activewindow>>foot,a,b",
                bus(Event::ActiveWindow {
                    class: "foot".into(),
                    title: "a,b".into(),
                    pid: 0,
                }),
            ),
            (
                "activewindowv2>>55d0a1",
                Some(HyprEvent::ActiveAddress("55d0a1".into())),
            ),
            ("activewindowv2>>", None),
            ("activewindowv2>>,", None),
            (
                "openwindow>>80e62df0,2,foot,t,1\n",
                bus(Event::OpenWindow {
                    address: "80e62df0".into(),
                    workspace: "2".into(),
                    class: "foot".into(),
                    title: "t,1".into(),
                }),
            ),
            ("openwindow>>80e62df0,2", None),
            (
                "closewindow>>80e62df0",
                bus(Event::CloseWindow {
                    address: "80e62df0".into(),
                }),
            ),
            ("workspace>>3", bus(Event::Workspace { name: "3".into() })),
            ("monitoradded>>DP-1", None),
            ("garbage", None),
        ];
        for (line, want) in cases {
            assert_eq!(parse_event(line), want, "{line}");
        }
    }

    #[test]
    fn active_pid_table() {
        for (json, want) in [
            (r#"{"address":"0x1","pid":4242,"class":"foot"}"#, Some(4242)),
            ("{}", None),
            (r#"{"pid":-1}"#, None),
            (r#"{"pid":0}"#, None),
            ("nope", None),
        ] {
            assert_eq!(active_pid(json), want, "{json}");
        }
    }
}

/// Drive the session from Hyprland events: frontmost pid -> QoS boost, bus events -> `publish`.
/// `lookup_pid` resolves the active window (normally `request("j/activewindow")` + [`active_pid`]);
/// its pid is filled into `Event::ActiveWindow`. `activewindowv2` repeats `activewindow` and is ignored.
pub fn run(
    events: impl Iterator<Item = HyprEvent>,
    mut lookup_pid: impl FnMut() -> Option<i32>,
    mut boost: impl FnMut(i32),
    mut publish: impl FnMut(&Event),
) {
    for e in events {
        let HyprEvent::Bus(mut ev) = e else { continue };
        if let Event::ActiveWindow { pid, .. } = &mut ev
            && let Some(p) = lookup_pid()
        {
            *pid = p;
            boost(p);
        }
        publish(&ev);
    }
}

#[cfg(test)]
mod run_tests {
    use super::*;

    #[test]
    fn active_window_gets_pid_boosts_and_everything_publishes() {
        let lines = [
            "activewindow>>foot,t",
            "activewindowv2>>a1",
            "closewindow>>a1",
            "activewindow>>x,y",
        ];
        let mut pids = [Some(10), None].into_iter();
        let mut boosted = Vec::new();
        let mut out = Vec::new();
        run(
            lines.iter().filter_map(|l| parse_event(l)),
            || pids.next().flatten(),
            |p| boosted.push(p),
            |e| out.push(e.clone()),
        );
        assert_eq!(boosted, vec![10]);
        assert_eq!(
            out,
            vec![
                Event::ActiveWindow {
                    class: "foot".into(),
                    title: "t".into(),
                    pid: 10
                },
                Event::CloseWindow {
                    address: "a1".into()
                },
                Event::ActiveWindow {
                    class: "x".into(),
                    title: "y".into(),
                    pid: 0
                },
            ]
        );
    }
}
