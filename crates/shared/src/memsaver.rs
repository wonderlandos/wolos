//! Memory Saver wire (ADR 0022): wol-realtime -> wold, one snapshot line per change on [`SOCKET`];
//! wold -> everyone, the [`Status`] JSON at [`STATUS`]. Inspired by memory-sentry (yanlvl99).
use serde::{Deserialize, Serialize};

/// wol-realtime -> wold (root): what the session shows and plays, and the user's choices.
pub const SOCKET: &str = "/var/run/wol-memsaver.sock";
/// wold -> Settings: what Memory Saver is doing (world readable, replaced atomically).
pub const STATUS: &str = "/var/run/wol-memsaver.json";
/// A snapshot line never grows past this (longer lines are dropped whole).
pub const MAX_LINE: usize = 64 * 1024;

fn items(v: &str) -> impl Iterator<Item = &str> {
    v.split(',').filter(|i| !i.is_empty())
}

/// One mapped window: its process, app id (window class, [`app_id`]) and whether it can be seen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub pid: i32,
    pub app: String,
    /// On a workspace a display shows, not minimized or hidden.
    pub visible: bool,
}

/// Everything wold needs from the session at one instant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub enabled: bool,
    /// App Nap: freeze apps hidden for a long time that do nothing (off by default).
    pub nap: bool,
    /// Frontmost process (0: none).
    pub front: i32,
    pub windows: Vec<Window>,
    /// Processes with a running audio or capture stream, or an MPRIS player that is Playing.
    pub playing: Vec<i32>,
    /// App ids the user excluded.
    pub exclude: Vec<String>,
}

/// Normalised app id: lower case, `[a-z0-9._-]` only (anything else becomes `_`), at most 64
/// bytes. Window classes and Settings exclusions compare through it; it is also scope-name safe.
pub fn app_id(class: &str) -> String {
    class
        .chars()
        .take(64)
        .map(|c| match c.to_ascii_lowercase() {
            c @ ('a'..='z' | '0'..='9' | '.' | '_' | '-') => c,
            _ => '_',
        })
        .collect()
}

fn list<T>(items: &[T], f: impl Fn(&T) -> String) -> String {
    items.iter().map(f).collect::<Vec<_>>().join(",")
}

impl Snapshot {
    /// `snapshot on nap=0 front=12 win=12:1:code,40:0:discord play=40 exclude=spotify\n`.
    pub fn line(&self) -> String {
        format!(
            "snapshot {} nap={} front={} win={} play={} exclude={}\n",
            if self.enabled { "on" } else { "off" },
            u8::from(self.nap),
            self.front.max(0),
            list(&self.windows, |w| format!(
                "{}:{}:{}",
                w.pid,
                u8::from(w.visible),
                app_id(&w.app)
            )),
            list(&self.playing, i32::to_string),
            list(&self.exclude, |e| app_id(e)),
        )
    }

    /// Inverse of [`Snapshot::line`]; None for anything malformed (the whole line is ignored).
    pub fn parse(line: &str) -> Option<Snapshot> {
        let mut f = line.trim_end_matches(['\r', '\n']).split(' ');
        if f.next()? != "snapshot" {
            return None;
        }
        let enabled = match f.next()? {
            "on" => true,
            "off" => false,
            _ => return None,
        };
        let mut s = Snapshot {
            enabled,
            ..Snapshot::default()
        };
        let pid = |v: &str| v.parse::<i32>().ok().filter(|p| *p > 0);
        for kv in f {
            let (k, v) = kv.split_once('=')?;
            match k {
                "nap" => s.nap = v == "1",
                "front" => s.front = v.parse::<i32>().ok()?.max(0),
                "win" => {
                    for w in items(v) {
                        let mut p = w.splitn(3, ':');
                        let (pid, vis, app) = (pid(p.next()?)?, p.next()?, p.next()?);
                        if app.is_empty() || app != app_id(app) {
                            return None;
                        }
                        s.windows.push(Window {
                            pid,
                            app: app.into(),
                            visible: vis == "1",
                        });
                    }
                }
                "play" => {
                    for p in items(v) {
                        s.playing.push(pid(p)?);
                    }
                }
                "exclude" => s.exclude = items(v).map(app_id).collect(),
                _ => {} // newer senders may add keys
            }
        }
        Some(s)
    }
}

/// What one app is going through, as Settings shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppState {
    /// In use (frontmost or visible).
    Active,
    /// In the background, inside the grace period.
    Waiting,
    /// Memory given back; ceilings follow what the app does.
    Saving,
    /// Paused (App Nap).
    Napping,
    /// The app needed its memory back while in the background (pressure): left alone until used.
    Released,
    /// Never touched: excluded by the user.
    Excluded,
    /// Never touched: playing sound or using the camera or microphone.
    Playing,
    /// Never touched: a game.
    Game,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppStatus {
    pub app: String,
    pub state: AppState,
    /// Memory this app gave back (bytes; 0 unless Saving/Napping).
    pub saved: u64,
    /// Memory it holds now (memory.current).
    pub memory: u64,
    /// Of which compressed away (memory.swap.current).
    pub swap: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub enabled: bool,
    /// Sum of [`AppStatus::saved`].
    pub saved: u64,
    pub apps: Vec<AppStatus>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snapshot {
        Snapshot {
            enabled: true,
            nap: false,
            front: 12,
            windows: vec![
                Window {
                    pid: 12,
                    app: "code".into(),
                    visible: true,
                },
                Window {
                    pid: 40,
                    app: "discord".into(),
                    visible: false,
                },
            ],
            playing: vec![40, 41],
            exclude: vec!["spotify".into()],
        }
    }

    #[test]
    fn snapshot_round_trips() {
        let s = snap();
        assert_eq!(
            s.line(),
            "snapshot on nap=0 front=12 win=12:1:code,40:0:discord play=40,41 exclude=spotify\n"
        );
        assert_eq!(Snapshot::parse(&s.line()), Some(s));
        let empty = Snapshot::default();
        assert_eq!(
            empty.line(),
            "snapshot off nap=0 front=0 win= play= exclude=\n"
        );
        assert_eq!(Snapshot::parse(&empty.line()), Some(empty));
    }

    #[test]
    fn app_ids_are_normalised_on_the_wire() {
        let mut s = snap();
        s.windows[0].app = "Code - OSS".into();
        s.exclude = vec!["com.Spotify Client".into()];
        let back = Snapshot::parse(&s.line()).unwrap();
        assert_eq!(back.windows[0].app, "code_-_oss");
        assert_eq!(back.exclude, vec!["com.spotify_client"]);
        assert_eq!(app_id(&"x".repeat(100)).len(), 64);
        assert_eq!(app_id("org.gnome.Nautilus"), "org.gnome.nautilus");
    }

    #[test]
    fn malformed_lines_are_ignored_whole() {
        for bad in [
            "",
            "foreground 12",
            "snapshot maybe",
            "snapshot on front=x",
            "snapshot on win=12:1",
            "snapshot on win=0:1:code",
            "snapshot on win=12:1:Code",
            "snapshot on play=-3",
            "snapshot on novalue",
        ] {
            assert_eq!(Snapshot::parse(bad), None, "{bad}");
        }
        // Unknown keys from a newer sender are skipped.
        assert!(Snapshot::parse("snapshot on future=1").is_some());
    }

    #[test]
    fn status_json_names_states_in_kebab_case() {
        let st = Status {
            enabled: true,
            saved: 5,
            apps: vec![AppStatus {
                app: "code".into(),
                state: AppState::Saving,
                saved: 5,
                memory: 10,
                swap: 3,
            }],
        };
        let j = serde_json::to_string(&st).unwrap();
        assert!(j.contains(r#""state":"saving""#), "{j}");
        assert_eq!(serde_json::from_str::<Status>(&j).unwrap(), st);
    }
}
