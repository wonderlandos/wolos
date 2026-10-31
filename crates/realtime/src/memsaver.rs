//! Memory Saver session side (ADR 0022): what Hyprland shows, what plays, and the user's choices,
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
        let name = shared::memsaver::app_id(inst.split('.').next().unwrap_or(inst));
        out.extend(
            windows
                .iter()
                .filter(|w| shared::memsaver::app_id(&w.app) == name)
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
                self.conn = UnixStream::connect(&self.socket).ok();
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

/// Session loop: a round on every Hyprland event (`kick`) and every 2 s. `hypr` answers IPC
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
        assert!(s.send(&snap));
        assert!(!s.send(&snap), "unchanged");
        drop(s);
        let mut got = String::new();
        l.accept().unwrap().0.read_to_string(&mut got).unwrap();
        assert_eq!(got, snap.line());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
