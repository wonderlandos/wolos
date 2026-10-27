//! Desktop widgets, pure side (ADR 0010, archicture/modules/widgets.md): grid placement, the
//! coalesced refresh schedule, the layout file store, and each built-in's data (weather, calendar,
//! music, batteries, system history). GTK lives in `ui/widgets*.rs`; parts in `kit::data`.
pub mod calendar;
pub mod eds;
pub mod media;
pub mod power;
pub mod schedule;
pub mod weather;

/// The cell grid is shared with `wol widgets` (placement without a screen).
pub use shared::widgets::grid;
/// The layout file store is shared with `wol widgets`.
pub use wol_core::widgets::{desktop_path, load_desktop as load, save_desktop as save};

use std::path::PathBuf;

/// `$XDG_CACHE_HOME`, else `~/.cache`.
pub fn cache_home() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| wol_core::widgets::home().join(".cache"))
}

/// Local date and time as widgets show it. `weekday` 0 = Monday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Now {
    pub y: i32,
    pub month: u32,
    pub day: u32,
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

impl Now {
    pub fn at(&self) -> calendar::DateTime {
        calendar::DateTime {
            y: self.y,
            m: self.month,
            d: self.day,
            hh: self.hour,
            mm: self.minute,
        }
    }
}

/// Hyprland events after which the active workspace may have gained or lost a fullscreen window.
pub fn fullscreen_may_change(event_line: &str) -> bool {
    let name = event_line.split(">>").next().unwrap_or("");
    matches!(
        name,
        "fullscreen"
            | "workspace"
            | "workspacev2"
            | "focusedmon"
            | "focusedmonv2"
            | "activespecial"
            | "closewindow"
            | "movewindow"
            | "movewindowv2"
    )
}

/// `hyprctl -j activeworkspace` -> whether a fullscreen window covers the desktop.
pub fn has_fullscreen(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v["hasfullscreen"].as_bool())
        .unwrap_or(false)
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Last `cap` samples (System widget charts).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    cap: usize,
    v: std::collections::VecDeque<f64>,
}

impl History {
    pub fn new(cap: usize) -> Self {
        History {
            cap,
            v: Default::default(),
        }
    }
    pub fn push(&mut self, x: f64) {
        if self.v.len() == self.cap {
            self.v.pop_front();
        }
        self.v.push_back(x);
    }
    pub fn values(&self) -> Vec<f64> {
        self.v.iter().copied().collect()
    }
    pub fn last(&self) -> Option<f64> {
        self.v.back().copied()
    }
}

/// "6.1 GB" from bytes (System widget).
pub fn gigabytes(bytes: i64) -> String {
    let g = bytes as f64 / 1_073_741_824.0;
    if g >= 10.0 {
        format!("{g:.0} GB")
    } else {
        format!("{g:.1} GB")
    }
}

/// "3 d 4 h", "5 h 12 min", "12 min".
pub fn uptime(secs: f64) -> String {
    let m = (secs / 60.0) as u64;
    let (d, h, m) = (m / 1440, m / 60 % 24, m % 60);
    match (d, h) {
        (0, 0) => format!("{m} min"),
        (0, _) => format!("{h} h {m} min"),
        _ => format!("{d} d {h} h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullscreen_pauses_follow_hyprland() {
        for (line, want) in [
            ("fullscreen>>1", true),
            ("workspacev2>>2,2", true),
            ("closewindow>>55d0", true),
            ("activewindow>>foot,~", false),
            ("openlayer>>wol-desktop", false),
            ("garbage", false),
        ] {
            assert_eq!(fullscreen_may_change(line), want, "{line}");
        }
        assert!(has_fullscreen(
            r#"{"id":1,"hasfullscreen":true,"windows":2}"#
        ));
        assert!(!has_fullscreen(r#"{"id":1,"hasfullscreen":false}"#));
        assert!(!has_fullscreen("not json"));
    }

    #[test]
    fn history_keeps_the_last_samples() {
        let mut h = History::new(3);
        for x in [1.0, 2.0, 3.0, 4.0] {
            h.push(x);
        }
        assert_eq!(h.values(), vec![2.0, 3.0, 4.0]);
        assert_eq!(h.last(), Some(4.0));
    }

    #[test]
    fn human_numbers() {
        assert_eq!(gigabytes(6_550_000_000), "6.1 GB");
        assert_eq!(gigabytes(17_179_869_184), "16 GB");
        for (s, want) in [
            (59.0, "0 min"),
            (720.0, "12 min"),
            (18_720.0, "5 h 12 min"),
            (273_600.0, "3 d 4 h"),
        ] {
            assert_eq!(uptime(s), want);
        }
    }
}
