//! The search index (ADR 0021): one memory-mapped segment per source, rebuilt only when its input
//! changed. A segment holds TSV records and a lowercase haystack; search scans the haystack with
//! memchr's `memmem`, ranks hits and groups them into one result per app.
pub mod parse;
pub mod search;
pub mod segment;

use serde::{Deserialize, Serialize};

/// What kind of source a segment indexes. The order is the tie-break when ranking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Pacman,
    Wol,
    Flatpak,
    Appimage,
    Recipe,
    Aur,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Pacman => "pacman",
            Kind::Wol => "wol",
            Kind::Flatpak => "flatpak",
            Kind::Appimage => "appimage",
            Kind::Recipe => "recipe",
            Kind::Aur => "aur",
        }
    }
    pub fn parse(s: &str) -> Option<Kind> {
        [
            Kind::Pacman,
            Kind::Wol,
            Kind::Flatpak,
            Kind::Appimage,
            Kind::Recipe,
            Kind::Aur,
        ]
        .into_iter()
        .find(|k| k.name() == s)
    }
}

/// One installable thing from one source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub kind: Kind,
    /// Repository or remote: `core`, `wolos`, `flathub`, `aur`, ...
    pub repo: String,
    /// Display name (pacman/AUR: the package name; Flatpak: the app name).
    pub name: String,
    /// What the installer takes: package name, Flatpak app id, `.wol` id, AppImage URL.
    pub id: String,
    pub version: String,
    pub summary: String,
    /// Grouping key: the same app from several sources shares it.
    pub key: String,
    /// Popularity hint (AUR votes; 0 elsewhere).
    #[serde(default)]
    pub votes: u32,
}

fn clean(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == '\t' || c == '\n' || c == '\r' {
                ' '
            } else {
                c
            }
        })
        .collect()
}

impl Record {
    pub fn to_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            self.kind.name(),
            clean(&self.repo),
            clean(&self.name),
            clean(&self.id),
            clean(&self.version),
            clean(&self.summary),
            clean(&self.key),
            self.votes
        )
    }

    pub fn from_line(line: &str) -> Option<Record> {
        let f: Vec<&str> = line.trim_end_matches('\n').split('\t').collect();
        if f.len() != 8 {
            return None;
        }
        Some(Record {
            kind: Kind::parse(f[0])?,
            repo: f[1].into(),
            name: f[2].into(),
            id: f[3].into(),
            version: f[4].into(),
            summary: f[5].into(),
            key: f[6].into(),
            votes: f[7].parse().unwrap_or(0),
        })
    }

    /// Lowercase text search looks at: name, id, summary.
    pub fn haystack(&self) -> String {
        format!("{} {} {}", self.name, self.id, self.summary).to_lowercase()
    }

    /// Source order (lower first): `[wolos]`, optimized builds, Arch, `.wol`, Flathub, AppImage,
    /// recipes, AUR.
    pub fn source_rank(&self) -> u8 {
        match self.kind {
            Kind::Pacman if self.repo == "wolos" => 0,
            Kind::Pacman
                if self.repo.starts_with("wolos-cachy") || self.repo.starts_with("cachyos") =>
            {
                1
            }
            Kind::Pacman => 2,
            Kind::Wol => 3,
            Kind::Flatpak => 4,
            Kind::Appimage => 5,
            Kind::Recipe => 6,
            Kind::Aur => 7,
        }
    }
}

/// Words a Flatpak id often ends with that do not name the app.
const GENERIC: [&str; 8] = [
    "client",
    "desktop",
    "app",
    "application",
    "gui",
    "main",
    "linux",
    "launcher",
];

/// Grouping key from a package name: lowercase, AUR packaging suffixes dropped.
pub fn package_key(name: &str) -> String {
    let n = name.to_lowercase();
    for suf in ["-bin", "-git", "-appimage", "-nightly", "-beta", "-stable"] {
        if let Some(s) = n.strip_suffix(suf)
            && !s.is_empty()
        {
            return s.to_owned();
        }
    }
    n
}

/// Grouping key of a Flatpak app: the id's last part (`org.mozilla.firefox` -> `firefox`) unless that
/// is a generic word, then the display name without spaces.
pub fn flatpak_key(id: &str, name: &str) -> String {
    let last = id.rsplit('.').next().unwrap_or(id).to_lowercase();
    if !last.is_empty() && !GENERIC.contains(&last.as_str()) {
        return last;
    }
    name.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_roundtrip_and_sanitise() {
        let r = Record {
            kind: Kind::Flatpak,
            repo: "flathub".into(),
            name: "Fire\tfox".into(),
            id: "org.mozilla.firefox".into(),
            version: "1".into(),
            summary: "Web\nbrowser".into(),
            key: "firefox".into(),
            votes: 0,
        };
        let back = Record::from_line(&r.to_line()).unwrap();
        assert_eq!(back.name, "Fire fox");
        assert_eq!(back.summary, "Web browser");
        assert_eq!(Record::from_line("junk"), None);
    }

    #[test]
    fn keys_group_the_same_app() {
        let cases = [
            (package_key("Visual-Studio-Code-Bin"), "visual-studio-code"),
            (package_key("paru-git"), "paru"),
            (package_key("-bin"), "-bin"),
            (flatpak_key("org.mozilla.firefox", "Firefox"), "firefox"),
            (flatpak_key("com.spotify.Client", "Spotify"), "spotify"),
            (flatpak_key("org.gnome.Desktop", "GNOME Desk"), "gnome-desk"),
        ];
        for (got, want) in cases {
            assert_eq!(got, want);
        }
    }
}
