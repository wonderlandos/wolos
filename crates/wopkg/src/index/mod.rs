// SPDX-License-Identifier: GPL-2.0-or-later
//! The search index (ADR 0021): one memory-mapped segment per source, rebuilt only when its input
//! changed. A segment holds TSV records and a lowercase haystack; search scans the haystack with
//! memchr's `memmem`, ranks hits and groups them into one result per app.
pub mod parse;
pub mod plugins;
pub mod search;
pub mod segment;
pub mod shards;

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
    /// Arch's AppStream: which package is which app. Names an app, installs nothing itself.
    Catalog,
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
            Kind::Catalog => "catalog",
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
            Kind::Catalog,
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
    /// Display name (pacman/community packages: the package name; Flatpak: the app name).
    pub name: String,
    /// What the installer takes: package name, Flatpak app id, `.wol` id, AppImage URL.
    pub id: String,
    pub version: String,
    pub summary: String,
    /// Grouping key: the same app from several sources shares it.
    pub key: String,
    /// Popularity hint (community package votes; 0 elsewhere).
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
    /// recipes, the community package collection.
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
            Kind::Catalog => 6,
            Kind::Aur => 7,
        }
    }
}

/// Grouping key from a package name: lowercase, community packaging suffixes dropped.
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

/// Grouping key of anything with an AppStream id: the id itself, so that two sources are one app
/// only when their metadata says so. A name alone never joins them.
pub fn app_key(id: &str) -> String {
    id.trim_end_matches(".desktop").to_lowercase()
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
    fn an_app_key_is_its_appstream_id() {
        assert_eq!(app_key("org.mozilla.firefox"), "org.mozilla.firefox");
        assert_eq!(
            app_key("org.gnome.Calculator.desktop"),
            "org.gnome.calculator"
        );
    }
}
