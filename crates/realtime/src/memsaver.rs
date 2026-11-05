// SPDX-License-Identifier: GPL-2.0-or-later
//! Memory Saver session side (ADR 0026): what the compositor shows, what plays, and the user's choices,
//! sent to wold as one `shared::memsaver` snapshot line whenever it changes.
use shared::memsaver::{Snapshot, Window};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// Windows from `j/clients` + `j/monitors`: visible when mapped, not hidden, and on a workspace
/// some display shows (its active one, or an open special workspace). Minimized windows live on a
/// special workspace nobody shows, so they count as hidden.
pub fn windows(clients: &str, monitors: &str) -> Vec<Window> {
    let shown: Vec<i64> = serde_json::from_str::<serde_json::Value>(monitors)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .flat_map(|m| {
            ["activeWorkspace", "specialWorkspace"]
                .map(|k| m.get(k).and_then(|w| w.get("id")).and_then(|i| i.as_i64()))
        })
        .flatten()
        .filter(|id| *id != 0)
        .collect();
    let Ok(serde_json::Value::Array(cs)) = serde_json::from_str(clients) else {
        return Vec::new();
    };
    cs.iter()
        .filter_map(|c| {
            let pid = i32::try_from(c.get("pid")?.as_i64()?)
                .ok()
                .filter(|p| *p > 0)?;
            let mapped = c.get("mapped").and_then(|m| m.as_bool()).unwrap_or(true);
            let hidden = c.get("hidden").and_then(|m| m.as_bool()).unwrap_or(false);
            let ws = c
                .get("workspace")
                .and_then(|w| w.get("id"))
                .and_then(|i| i.as_i64());
            let class = c.get("class").and_then(|s| s.as_str()).unwrap_or("");
            let app = if class.is_empty() { "app" } else { class };
            Some(Window {
                pid,
                app: app.into(),
                visible: mapped && !hidden && ws.is_some_and(|w| shown.contains(&w)),
            })
        })
        .collect()
}

/// pids with a running (uncorked) stream from `pactl -f json list sink-inputs|source-outputs`:
/// playing sound, or recording (calls).
pub fn streams(pactl_json: &str) -> Vec<i32> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str(pactl_json) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|s| !s.get("corked").and_then(|c| c.as_bool()).unwrap_or(false))
        .filter_map(|s| {
            s.get("properties")?
                .get("application.process.id")?
                .as_str()?
                .parse()
                .ok()
        })
        .collect()
}

/// Words of a reverse-DNS class that name no app: the vendor or the forge, not the program.
const VENDOR_WORDS: [&str; 9] = [
    "gnome",
    "kde",
    "freedesktop",
    "github",
    "gitlab",
    "flathub",
    "mozilla",
    "google",
    "microsoft",
];

/// A Flatpak app's window class is its app id (`com.spotify.Client`) and its MPRIS name the short
/// one (`spotify`): `name` is a word of the class after its first (the top-level domain), and not a
/// vendor word.
fn short_name_of(class: &str, name: &str) -> bool {
    let class = shared::memsaver::app_id(class);
    let parts: Vec<&str> = class.split('.').collect();
    parts.len() >= 3 && !VENDOR_WORDS.contains(&name) && parts[1..].contains(&name)
}

/// pids of MPRIS players that are Playing, from `playerctl -a metadata --format
/// '{{status}}\t{{playerInstance}}'` lines. Chromium-style instance names carry the pid
/// (`chromium.instance4242`); others match a window's app id (`spotify`).
pub fn players(playerctl: &str, windows: &[Window]) -> Vec<i32> {
    let mut out = Vec::new();
    for l in playerctl.lines() {
        let Some(("Playing", inst)) = l.split_once('\t') else {
            continue;
        };
        let inst = inst.trim();
        if let Some(pid) = inst
            .rsplit_once(".instance")
            .and_then(|(_, p)| p.parse::<i32>().ok())
        {
            out.push(pid);
            continue;
        }
        // A dotted class names the app whole (`org.wolos.webapp.music`, QA-100 H9-20); only an
        // instance suffix that names no window (`spotify.instance_2`) falls back to its first word.
        let whole = shared::memsaver::app_id(inst);
        let first = shared::memsaver::app_id(inst.split('.').next().unwrap_or(inst));
        let named = |name: &str, w: &Window| shared::memsaver::app_id(&w.app) == name;
        let name = if windows.iter().any(|w| named(&whole, w)) {
            whole
        } else {
            first
        };
        out.extend(
            windows
                .iter()
                .filter(|w| named(&name, w) || short_name_of(&w.app, &name))
                .map(|w| w.pid),
        );
    }
    out
}

/// Snapshot sender: one connection to wold, reconnecting; only changes go out.
pub struct Sender {
    socket: PathBuf,
    conn: Option<UnixStream>,
    last: Option<String>,
}

impl Default for Sender {
    fn default() -> Self {
        Self::at(shared::memsaver::SOCKET)
    }
}

impl Sender {
    pub fn at(socket: impl Into<PathBuf>) -> Self {
        Sender {
            socket: socket.into(),
            conn: None,
            last: None,
        }
    }

    /// Send `s` if it differs from the last one sent. True when written.
    pub fn send(&mut self, s: &Snapshot) -> bool {
        let line = s.line();
        if self.last.as_deref() == Some(&line) {
            return false;
        }
        for _ in 0..2 {
            if self.conn.is_none() {
                self.conn = shared::jobs::connect(&self.socket).ok();
            }
            match self.conn.as_mut().map(|c| c.write_all(line.as_bytes())) {
                Some(Ok(())) => {
                    self.last = Some(line);
                    return true;
                }
                Some(Err(_)) => self.conn = None,
                None => return false,
            }
        }
        false
    }
}

/// Sound and players are looked at every this many rounds (a round is at most 2 s).
const MEDIA_EVERY: u32 = 2;

/// Session loop: a round on every compositor event (`kick`) and every 2 s. `hypr` answers IPC
/// requests, `run` runs a command and returns its stdout; prefs are reread every round.
pub fn run(
    kick: std::sync::mpsc::Receiver<()>,
    mut hypr: impl FnMut(&str) -> Option<String>,
    mut run: impl FnMut(&[&str]) -> Option<String>,
    config_home: PathBuf,
) {
    let mut sender = Sender::default();
    let mut playing = Vec::new();
    let mut round = 0u32;
    loop {
        match kick.recv_timeout(std::time::Duration::from_secs(2)) {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            _ => while kick.try_recv().is_ok() {},
        }
        let (Some(clients), Some(monitors)) = (hypr("j/clients"), hypr("j/monitors")) else {
            continue;
        };
        let w = windows(&clients, &monitors);
        if round.is_multiple_of(MEDIA_EVERY) {
            playing.clear();
            for kind in ["sink-inputs", "source-outputs"] {
                if let Some(j) = run(&["pactl", "-f", "json", "list", kind]) {
                    playing.extend(streams(&j));
                }
            }
            let fmt = "{{status}}\t{{playerInstance}}";
            if let Some(out) = run(&["playerctl", "-a", "metadata", "--format", fmt]) {
                playing.extend(players(&out, &w));
            }
            playing.sort_unstable();
            playing.dedup();
        }
        round = round.wrapping_add(1);
        let prefs = wol_core::memsaver::load(&config_home);
        let front = hypr("j/activewindow")
            .and_then(|j| crate::hypr::active_pid(&j))
            .unwrap_or(0);
        sender.send(&Snapshot {
            enabled: prefs.enabled,
            nap: prefs.nap,
            front,
            windows: w,
            playing: playing.clone(),
            exclude: prefs.exclude,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENTS: &str = r#"[
        {"pid": 20, "class": "code", "mapped": true, "hidden": false, "workspace": {"id": 1}},
        {"pid": 30, "class": "discord", "mapped": true, "hidden": false, "workspace": {"id": 2}},
        {"pid": 40, "class": "Spotify", "mapped": true, "hidden": false, "workspace": {"id": -98, "name": "special:minimized"}},
        {"pid": 50, "class": "foot", "mapped": true, "hidden": true, "workspace": {"id": 1}},
        {"pid": 60, "class": "", "mapped": true, "hidden": false, "workspace": {"id": 3}},
        {"pid": 0, "class": "ghost", "workspace": {"id": 1}}
    ]"#;
    const MONITORS: &str = r#"[
        {"name": "eDP-1", "activeWorkspace": {"id": 1}, "specialWorkspace": {"id": 0}},
        {"name": "DP-1", "activeWorkspace": {"id": 3}, "specialWorkspace": {"id": 0}}
    ]"#;

    #[test]
    fn visible_means_on_a_shown_workspace_and_not_hidden_or_minimized() {
        let w = windows(CLIENTS, MONITORS);
        let vis: Vec<(i32, &str, bool)> = w
            .iter()
            .map(|w| (w.pid, w.app.as_str(), w.visible))
            .collect();
        assert_eq!(
            vis,
            vec![
                (20, "code", true),
                (30, "discord", false),
                (40, "Spotify", false),
                (50, "foot", false),
                (60, "app", true),
            ]
        );
        assert!(windows("nope", MONITORS).is_empty());
        // An open special workspace (scratchpad) is shown.
        let m = r#"[{"activeWorkspace": {"id": 2}, "specialWorkspace": {"id": -98}}]"#;
        assert!(
            windows(CLIENTS, m)
                .iter()
                .find(|w| w.pid == 40)
                .unwrap()
                .visible
        );
    }

    #[test]
    fn running_streams_count_and_corked_ones_do_not() {
        let j = r#"[
            {"corked": false, "properties": {"application.process.id": "41"}},
            {"corked": true, "properties": {"application.process.id": "77"}},
            {"corked": false, "properties": {"application.name": "no pid"}}
        ]"#;
        assert_eq!(streams(j), vec![41]);
        assert!(streams("").is_empty());
    }

    #[test]
    fn playing_players_name_their_process() {
        let w = windows(CLIENTS, MONITORS);
        let out = "Playing\tchromium.instance4242\nPaused\tfirefox.instance_1_9\nPlaying\tspotify\nStopped\tcode\n";
        assert_eq!(players(out, &w), vec![4242, 40]);
    }

    /// QA-100 H9-20: a reverse-DNS class (a web app, a Flatpak) is compared whole, not by its
    /// first word `org`, so a playing web app is kept and another app of the same vendor is not.
    #[test]
    fn dotted_exclusion_whole_class() {
        let w = vec![
            Window {
                pid: 50,
                app: "org.wolos.webapp.music".into(),
                visible: false,
            },
            Window {
                pid: 51,
                app: "org.wolos.webapp.doctor".into(),
                visible: false,
            },
            Window {
                pid: 52,
                app: "spotify".into(),
                visible: false,
            },
        ];
        let out = "Playing\torg.wolos.webapp.music\nPlaying\tspotify.instance_2\n";
        assert_eq!(players(out, &w), vec![50, 52]);
    }

    /// IMAGE-F: a Flatpak app's window class is its app id (`com.spotify.Client`) while its MPRIS
    /// name is the short one (`org.mpris.MediaPlayer2.spotify`, which playerctl prints as
    /// `spotify`). Without the match the player is not found, and the Flatpak's sound stream cannot
    /// stand in (it reports the pid inside the sandbox): music would stop when the window is hidden.
    #[test]
    fn a_flatpak_player_is_found_by_the_short_name_of_its_app_id() {
        let win = |pid: i32, app: &str| Window {
            pid,
            app: app.into(),
            visible: false,
        };
        let w = vec![
            win(60, "com.spotify.Client"),
            win(61, "org.mozilla.firefox"),
            win(62, "org.gnome.Nautilus"),
            win(63, "io.github.celluloid_player.Celluloid"),
        ];
        let out = "Playing\tspotify\nPlaying\tfirefox.instance_1_9\nPlaying\tcelluloid\nPaused\tnautilus\n";
        assert_eq!(players(out, &w), vec![60, 61, 63]);
        // A vendor word is not an app: "mozilla" or "gnome" name no player, and "github" no one.
        assert!(players("Playing\tgnome\nPlaying\tgithub\nPlaying\tmozilla\n", &w).is_empty());
        // A window with exactly that name and a Flatpak's both stay: when unsure, nothing playing is frozen.
        let w2 = vec![win(70, "spotify"), win(71, "com.spotify.Client")];
        assert_eq!(players("Playing\tspotify\n", &w2), vec![70, 71]);
    }

    #[test]
    fn sender_sends_changes_only_and_survives_wold_being_away() {
        use std::io::Read;
        let dir = std::env::temp_dir().join(format!("wol-ms-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.sock");
        let _ = std::fs::remove_file(&path);
        let mut s = Sender::at(&path);
        let snap = Snapshot {
            enabled: true,
            front: 20,
            ..Snapshot::default()
        };
        assert!(!s.send(&snap), "wold not running");
        let l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let wold = std::thread::spawn(move || {
            let (c, _) = l.accept().unwrap();
            shared::jobs::greet(&c).unwrap();
            let mut got = String::new();
            (&c).read_to_string(&mut got).unwrap();
            got
        });
        assert!(s.send(&snap));
        assert!(!s.send(&snap), "unchanged");
        drop(s);
        let got = wold.join().unwrap();
        assert_eq!(got, snap.line());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
