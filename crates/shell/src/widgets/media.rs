//! Music widget: what an MPRIS player (session bus) is playing, with play/pause/next/previous.
//! Event-driven: a worker thread sleeps in the bus until a player changes; nothing polls.
//! Album art is shown only from local files (`file://`); remote art URLs are never fetched.
use std::collections::HashMap;
use std::path::PathBuf;
use zbus::zvariant::OwnedValue;

pub const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Playing {
    /// App name ("Firefox").
    pub player: String,
    /// Bus name, for the controls.
    pub bus: String,
    pub title: String,
    pub artist: String,
    pub art: Option<PathBuf>,
    pub playing: bool,
    pub can_next: bool,
    pub can_prev: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    Stopped,
}

impl Status {
    pub fn parse(s: &str) -> Status {
        match s {
            "Playing" => Status::Playing,
            "Paused" => Status::Paused,
            _ => Status::Stopped,
        }
    }
}

/// The player the widget follows: one playing, else one paused with a title.
pub fn pick(players: &[(Status, bool)]) -> Option<usize> {
    players
        .iter()
        .position(|(s, _)| *s == Status::Playing)
        .or_else(|| {
            players
                .iter()
                .position(|(s, titled)| *s == Status::Paused && *titled)
        })
}

/// `file:///home/a/Cover%20Art.png` -> path; any other scheme -> None (no network fetches).
pub fn art_path(url: &str) -> Option<PathBuf> {
    let p = url.strip_prefix("file://")?;
    let bytes = p.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(h) = p
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// Player name from the bus name when the player gives no Identity: `org.mpris.MediaPlayer2.vlc` -> "Vlc".
pub fn name_from_bus(bus: &str) -> String {
    let id = bus
        .strip_prefix(PREFIX)
        .unwrap_or(bus)
        .split('.')
        .next()
        .unwrap_or("");
    let mut c = id.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn get<T: TryFrom<OwnedValue>>(m: &HashMap<String, OwnedValue>, k: &str) -> Option<T> {
    m.get(k)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| T::try_from(v).ok())
}

fn read_player(conn: &zbus::blocking::Connection, bus: &str) -> Option<(Status, Playing)> {
    let props = zbus::blocking::fdo::PropertiesProxy::builder(conn)
        .destination(bus)
        .ok()?
        .path(PATH)
        .ok()?
        .build()
        .ok()?;
    let p: HashMap<String, OwnedValue> = props
        .get_all(zbus::names::InterfaceName::try_from(PLAYER).ok()?)
        .ok()?;
    let status = Status::parse(&get::<String>(&p, "PlaybackStatus").unwrap_or_default());
    let meta: HashMap<String, OwnedValue> = get(&p, "Metadata").unwrap_or_default();
    let identity = props
        .get(
            zbus::names::InterfaceName::try_from("org.mpris.MediaPlayer2").ok()?,
            "Identity",
        )
        .ok()
        .and_then(|v| String::try_from(v).ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| name_from_bus(bus));
    Some((
        status,
        Playing {
            player: identity,
            bus: bus.to_string(),
            title: get(&meta, "xesam:title").unwrap_or_default(),
            artist: get::<Vec<String>>(&meta, "xesam:artist")
                .map(|a| a.join(", "))
                .unwrap_or_default(),
            art: get::<String>(&meta, "mpris:artUrl").and_then(|u| art_path(&u)),
            playing: status == Status::Playing,
            can_next: get(&p, "CanGoNext").unwrap_or(false),
            can_prev: get(&p, "CanGoPrevious").unwrap_or(false),
        },
    ))
}

fn current(conn: &zbus::blocking::Connection) -> Option<Playing> {
    let dbus = zbus::blocking::fdo::DBusProxy::new(conn).ok()?;
    let mut names: Vec<String> = dbus
        .list_names()
        .ok()?
        .into_iter()
        .map(|n| n.to_string())
        .filter(|n| n.starts_with(PREFIX))
        .collect();
    names.sort();
    let players: Vec<(Status, Playing)> =
        names.iter().filter_map(|b| read_player(conn, b)).collect();
    let keys: Vec<(Status, bool)> = players
        .iter()
        .map(|(s, p)| (*s, !p.title.is_empty()))
        .collect();
    pick(&keys).map(|i| players[i].1.clone())
}

/// Sends the current track now and after every player change, until the receiver is dropped.
/// Ends quietly without a session bus (the widget then says nothing is playing).
pub fn watch(tx: async_channel::Sender<Option<Playing>>) {
    let _ = std::thread::Builder::new()
        .name("wol-mpris".into())
        .spawn(move || {
            let Ok(conn) = zbus::blocking::Connection::session() else {
                let _ = tx.send_blocking(None);
                return;
            };
            let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else {
                return;
            };
            let rules = [
                format!(
                    "type='signal',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged',path='{PATH}'"
                ),
                "type='signal',interface='org.freedesktop.DBus',member='NameOwnerChanged',arg0namespace='org.mpris.MediaPlayer2'".into(),
            ];
            for r in rules {
                if let Ok(rule) = zbus::MatchRule::try_from(r.as_str()) {
                    let _ = dbus.add_match_rule(rule);
                }
            }
            let mut last = current(&conn);
            if tx.send_blocking(last.clone()).is_err() {
                return;
            }
            for msg in zbus::blocking::MessageIterator::from(&conn) {
                let Ok(msg) = msg else { break };
                if msg.header().message_type() != zbus::message::Type::Signal {
                    continue;
                }
                let now = current(&conn);
                if now != last {
                    last = now.clone();
                    if tx.send_blocking(now).is_err() {
                        break;
                    }
                }
                if tx.is_closed() {
                    break;
                }
            }
        });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    PlayPause,
    Next,
    Previous,
}

/// One control call, off the UI thread.
pub fn control(bus: &str, c: Control) {
    let bus = bus.to_string();
    let method = match c {
        Control::PlayPause => "PlayPause",
        Control::Next => "Next",
        Control::Previous => "Previous",
    };
    let _ = std::thread::Builder::new()
        .name("wol-mpris-ctl".into())
        .spawn(move || {
            if let Ok(conn) = zbus::blocking::Connection::session()
                && let Err(e) =
                    conn.call_method(Some(bus.as_str()), PATH, Some(PLAYER), method, &())
            {
                eprintln!("wol-shell: music {method}: {e}");
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_playing_player_then_a_paused_one() {
        use Status::*;
        for (players, want) in [
            (vec![(Paused, true), (Playing, true)], Some(1)),
            (vec![(Stopped, true), (Paused, true)], Some(1)),
            (vec![(Paused, false), (Stopped, true)], None),
            (vec![], None),
        ] {
            assert_eq!(pick(&players), want, "{players:?}");
        }
        assert_eq!(Status::parse("Playing"), Playing);
        assert_eq!(Status::parse("x"), Stopped);
    }

    #[test]
    fn art_only_from_local_files() {
        assert_eq!(
            art_path("file:///home/a/Cover%20Art.png"),
            Some(PathBuf::from("/home/a/Cover Art.png"))
        );
        assert_eq!(art_path("https://i.scdn.co/image/ab67"), None);
        assert_eq!(art_path("data:image/png;base64,x"), None);
        assert_eq!(name_from_bus("org.mpris.MediaPlayer2.vlc"), "Vlc");
        assert_eq!(
            name_from_bus("org.mpris.MediaPlayer2.brave.instance77"),
            "Brave"
        );
    }

    /// Manual: `cargo test -p shell --lib live_mpris -- --ignored --nocapture` prints what plays now.
    #[test]
    #[ignore]
    fn live_mpris() {
        let conn = zbus::blocking::Connection::session().unwrap();
        println!("{:#?}", current(&conn));
    }
}
