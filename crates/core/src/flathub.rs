// SPDX-License-Identifier: BSD-2-Clause
//! WolOS's curation of Flathub, the second source of the App Store (docs/modules/flathub.md).
//! Flathub has everything; the Store should say what it knows about each app and put the apps
//! worth opening first. Pure, apart from [`warm`], which reads Flathub's local AppStream once.
//!
//! Three standings, named for what was done and never for "safe" (the person always sees the
//! findings, never the word: the hub's supply-chain policy, hub repo
//! `docs/adr/0049-hub-supply-chain-policy.md`, not in this tree yet):
//!
//! - [`Standing::Picked`]: WolOS chose it for the list below. A choice, not a code audit. The hub's
//!   Trusted level needs a reviewer on every release; this is the shelf, not that badge.
//! - [`Standing::Verified`]: Flathub verified that the developer owns the app id. The hub's Origin
//!   level: it says who published, by an automatic proof.
//! - [`Standing::Community`]: neither; packaged by whoever submitted it. Like the hub's community
//!   level, the consent sheet (`wol-flatpak-consent`) still asks for every permission.
//!
//! A pick may or may not be verified (Spotify, Slack and Zoom are packaged by the community); the
//! details sentence says which. Nothing here widens what an app may do: permissions stay with the
//! consent helper (`crate::flatpak`).
use crate::flatpak::Hit;
use crate::store::Category;
use std::collections::HashSet;
use std::sync::OnceLock;

/// An app WolOS picked from Flathub.
#[derive(Clone, Copy, Debug)]
pub struct Curated {
    pub id: &'static str,
    pub name: &'static str,
    /// One line, sentence case, no full stop.
    pub summary: &'static str,
    pub category: Category,
}

const fn pick(
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    category: Category,
) -> Curated {
    Curated {
        id,
        name,
        summary,
        category,
    }
}

use Category::{Create, Develop, Play, Work};

/// The picks: apps people ask for that Arch does not carry (or carries as a build that lags),
/// each present on Flathub under the id below on 2026-11-04. None repeats a native catalog app
/// (the test fails if one does): one app, one tile.
pub const CURATED: [Curated; 22] = [
    // Work
    pick(
        "md.obsidian.Obsidian",
        "Obsidian",
        "Markdown-based knowledge base",
        Work,
    ),
    pick(
        "com.logseq.Logseq",
        "Logseq",
        "Outliner notes that link to each other",
        Work,
    ),
    pick(
        "net.cozic.joplin_desktop",
        "Joplin",
        "Notes and to-do lists that sync",
        Work,
    ),
    pick(
        "org.zotero.Zotero",
        "Zotero",
        "Collect, organize and cite research",
        Work,
    ),
    pick(
        "org.onlyoffice.desktopeditors",
        "ONLYOFFICE",
        "Documents, spreadsheets and slides",
        Work,
    ),
    pick(
        "com.bitwarden.desktop",
        "Bitwarden",
        "Password manager for all your devices",
        Work,
    ),
    pick("com.slack.Slack", "Slack", "Team chat for work", Work),
    pick("us.zoom.Zoom", "Zoom", "Video meetings", Work),
    pick(
        "com.dropbox.Client",
        "Dropbox",
        "Your files on every computer",
        Work,
    ),
    pick(
        "com.brave.Browser",
        "Brave",
        "Web browser that blocks ads and trackers",
        Work,
    ),
    pick(
        "com.google.Chrome",
        "Google Chrome",
        "The browser from Google",
        Work,
    ),
    pick("app.zen_browser.zen", "Zen", "Calm, fast web browser", Work),
    pick(
        "im.riot.Riot",
        "Element",
        "Chat on the Matrix network",
        Work,
    ),
    pick(
        "org.localsend.localsend_app",
        "LocalSend",
        "Send files to nearby devices",
        Work,
    ),
    pick(
        "com.rustdesk.RustDesk",
        "RustDesk",
        "Reach another computer from afar",
        Work,
    ),
    // Play
    pick("com.spotify.Client", "Spotify", "Music streaming", Play),
    pick(
        "com.discordapp.Discord",
        "Discord",
        "Voice, video and text chat for communities",
        Play,
    ),
    pick(
        "com.usebottles.bottles",
        "Bottles",
        "Run Windows programs in their own bottles",
        Play,
    ),
    pick(
        "com.heroicgameslauncher.hgl",
        "Heroic",
        "Play Epic, GOG and Amazon games",
        Play,
    ),
    // Develop
    pick(
        "io.dbeaver.DBeaverCommunity",
        "DBeaver",
        "One tool for every database",
        Develop,
    ),
    pick(
        "com.getpostman.Postman",
        "Postman",
        "Build and test web APIs",
        Develop,
    ),
    // Create
    pick(
        "com.github.tchx84.Flatseal",
        "Flatseal",
        "See and change what each Flatpak app may do",
        Create,
    ),
];

/// Where a Flathub app stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// In [`CURATED`]; `verified` says whether Flathub also verified the developer.
    Picked {
        verified: bool,
    },
    Verified,
    Community,
}

/// The pick with this app id.
pub fn curated(id: &str) -> Option<&'static Curated> {
    CURATED.iter().find(|c| c.id == id)
}

/// Where `id` stands, given Flathub's verified ids.
pub fn standing(id: &str, verified: &HashSet<String>) -> Standing {
    let v = verified.contains(id);
    if curated(id).is_some() {
        Standing::Picked { verified: v }
    } else if v {
        Standing::Verified
    } else {
        Standing::Community
    }
}

fn rank_of(s: Standing) -> u8 {
    match s {
        Standing::Picked { .. } => 0,
        Standing::Verified => 1,
        Standing::Community => 2,
    }
}

/// Picks first, then verified, then the rest; each group keeps the order `flatpak search` gave.
pub fn rank(mut hits: Vec<Hit>, verified: &HashSet<String>) -> Vec<Hit> {
    hits.sort_by_key(|h| rank_of(standing(&h.id, verified)));
    hits
}

/// The picks as search hits, for a shelf; `covered(hit)` drops those a native package covers.
pub fn curated_hits(covered: impl Fn(&Hit) -> bool) -> Vec<Hit> {
    CURATED
        .iter()
        .map(|c| Hit {
            id: c.id.into(),
            name: c.name.into(),
            summary: c.summary.into(),
        })
        .filter(|h| !covered(h))
        .collect()
}

static VERIFIED: OnceLock<HashSet<String>> = OnceLock::new();

/// Verified ids of the AppStream files at `paths` (the user's and the system's Flathub copy),
/// each read through `read`; a file it cannot read counts for nothing.
pub fn load_verified(paths: &[String], read: impl Fn(&str) -> Option<String>) -> HashSet<String> {
    let mut all = HashSet::new();
    for p in paths {
        if let Some(xml) = read(p) {
            all.extend(crate::appstream::verified_ids(&xml));
        }
    }
    all
}

/// Reads Flathub's local AppStream under `home` once per process, through `read` (the caller owns
/// the file system: the files are tens of megabytes, so call it off the UI thread); [`cached`]
/// never reads.
pub fn warm(home: &str, read: impl Fn(&str) -> Option<String>) -> &'static HashSet<String> {
    VERIFIED.get_or_init(|| {
        let xml: Vec<String> = crate::appstream::flathub_paths(home)
            .into_iter()
            .map(|(x, _)| x)
            .collect();
        load_verified(&xml, read)
    })
}

/// The verified set if [`warm`] ran, else empty: a tile built before then reads as community and
/// is relabelled by the next search.
pub fn cached() -> &'static HashSet<String> {
    static NONE: OnceLock<HashSet<String>> = OnceLock::new();
    VERIFIED
        .get()
        .unwrap_or_else(|| NONE.get_or_init(HashSet::new))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_verified_unions_files_and_skips_unreadable_ones() {
        let item = |id: &str| {
            format!(
                "<component><id>{id}</id><custom><value key=\"flathub::verification::verified\">true</value></custom></component>"
            )
        };
        let files = [("a.xml", item("org.a.A")), ("b.xml", item("org.b.B"))];
        let read = |p: &str| files.iter().find(|f| f.0 == p).map(|f| f.1.clone());
        let paths = ["a.xml", "missing.xml", "b.xml"].map(String::from);
        let v = load_verified(&paths, read);
        assert_eq!(v.len(), 2);
        assert!(v.contains("org.a.A") && v.contains("org.b.B"));
    }

    #[test]
    fn warm_reads_the_flathub_appstream_of_the_home_it_is_given() {
        let seen = std::cell::RefCell::new(Vec::new());
        let v = warm("/home/ann", |p| {
            seen.borrow_mut().push(p.to_owned());
            None
        });
        // Once per process: another test in this binary may have warmed it first, then nothing is read.
        assert!(v.is_empty());
        assert!(
            seen.borrow().is_empty()
                || seen.borrow()[0]
                    .starts_with("/home/ann/.local/share/flatpak/appstream/flathub/")
        );
        assert!(cached().is_empty());
    }
}
