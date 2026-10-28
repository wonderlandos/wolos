//! App Store catalog: curated desktop apps mapped to Arch packages, install plans for wol-admind
//! (`pkg-install`), `pacman -Ss` parsing. Pure; the shell runs reads off the UI thread.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Create,
    Work,
    Play,
    Develop,
}

impl Category {
    pub const ALL: [Category; 4] = [
        Category::Create,
        Category::Work,
        Category::Play,
        Category::Develop,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Category::Create => "Create",
            Category::Work => "Work",
            Category::Play => "Play",
            Category::Develop => "Develop",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct App {
    pub id: &'static str,
    pub name: &'static str,
    /// One line, sentence case, no full stop.
    pub summary: &'static str,
    pub category: Category,
    /// freedesktop / WolOS icon name.
    pub icon: &'static str,
    /// .desktop basename (no extension): Open launches it, its presence means installed.
    pub desktop: &'static str,
    pub packages: &'static [&'static str],
}

const fn app(
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    category: Category,
    icon: &'static str,
    desktop: &'static str,
    packages: &'static [&'static str],
) -> App {
    App {
        id,
        name,
        summary,
        category,
        icon,
        desktop,
        packages,
    }
}

use Category::*;

pub const CATALOG: [App; 40] = [
    // Work
    app(
        "firefox",
        "Firefox",
        "Fast, private web browser",
        Work,
        "firefox",
        "firefox",
        &["firefox"],
    ),
    app(
        "thunderbird",
        "Thunderbird",
        "Mail, calendar and contacts",
        Work,
        "thunderbird",
        "thunderbird",
        &["thunderbird"],
    ),
    app(
        "libreoffice",
        "LibreOffice",
        "Documents, spreadsheets and slides",
        Work,
        "libreoffice-startcenter",
        "libreoffice-startcenter",
        &["libreoffice-fresh"],
    ),
    app(
        "chromium",
        "Chromium",
        "Open-source web browser",
        Work,
        "chromium",
        "chromium-browser",
        &["chromium"],
    ),
    app(
        "okular",
        "Okular",
        "Read PDFs and e-books",
        Work,
        "okular",
        "org.kde.okular",
        &["okular"],
    ),
    app(
        "keepassxc",
        "KeePassXC",
        "Keep your passwords safe",
        Work,
        "keepassxc",
        "org.keepassxc.KeePassXC",
        &["keepassxc"],
    ),
    app(
        "signal",
        "Signal",
        "Private messages and calls",
        Work,
        "signal-desktop",
        "signal-desktop",
        &["signal-desktop"],
    ),
    app(
        "telegram",
        "Telegram",
        "Fast messaging in the cloud",
        Work,
        "telegram",
        "org.telegram.desktop",
        &["telegram-desktop"],
    ),
    app(
        "nextcloud",
        "Nextcloud",
        "Sync files with your cloud",
        Work,
        "Nextcloud",
        "com.nextcloud.desktopclient.nextcloud",
        &["nextcloud-client"],
    ),
    app(
        "evolution",
        "Evolution",
        "Mail and calendar for teams",
        Work,
        "evolution",
        "org.gnome.Evolution",
        &["evolution"],
    ),
    // Create
    app(
        "gimp",
        "GIMP",
        "Edit photos and images",
        Create,
        "gimp",
        "gimp",
        &["gimp"],
    ),
    app(
        "inkscape",
        "Inkscape",
        "Vector illustration and design",
        Create,
        "org.inkscape.Inkscape",
        "org.inkscape.Inkscape",
        &["inkscape"],
    ),
    app(
        "krita",
        "Krita",
        "Paint and draw like a pro",
        Create,
        "krita",
        "org.kde.krita",
        &["krita"],
    ),
    app(
        "blender",
        "Blender",
        "3D modelling, animation and film",
        Create,
        "blender",
        "blender",
        &["blender"],
    ),
    app(
        "darktable",
        "darktable",
        "Develop RAW photos",
        Create,
        "darktable",
        "org.darktable.darktable",
        &["darktable"],
    ),
    app(
        "kdenlive",
        "Kdenlive",
        "Edit video on a timeline",
        Create,
        "kdenlive",
        "org.kde.kdenlive",
        &["kdenlive"],
    ),
    app(
        "shotcut",
        "Shotcut",
        "Simple video editing",
        Create,
        "shotcut",
        "org.shotcut.Shotcut",
        &["shotcut"],
    ),
    app(
        "audacity",
        "Audacity",
        "Record and edit audio",
        Create,
        "audacity",
        "audacity",
        &["audacity"],
    ),
    app(
        "obs",
        "OBS Studio",
        "Record and stream your screen",
        Create,
        "com.obsproject.Studio",
        "com.obsproject.Studio",
        &["obs-studio"],
    ),
    app(
        "ardour",
        "Ardour",
        "Record, mix and master music",
        Create,
        "ardour",
        "ardour8",
        &["ardour"],
    ),
    app(
        "scribus",
        "Scribus",
        "Page layout and publishing",
        Create,
        "scribus",
        "scribus",
        &["scribus"],
    ),
    // Play
    app(
        "steam",
        "Steam",
        "Thousands of games, one library",
        Play,
        "steam",
        "steam",
        &["steam"],
    ),
    app(
        "vlc",
        "VLC",
        "Plays any video or music file",
        Play,
        "vlc",
        "vlc",
        &["vlc"],
    ),
    app(
        "mpv",
        "mpv",
        "Minimal, high-quality video player",
        Play,
        "mpv",
        "mpv",
        &["mpv"],
    ),
    app(
        "strawberry",
        "Strawberry",
        "Your music collection",
        Play,
        "strawberry",
        "org.strawberrymusicplayer.strawberry",
        &["strawberry"],
    ),
    app(
        "supertuxkart",
        "SuperTuxKart",
        "Kart racing with friends",
        Play,
        "supertuxkart",
        "supertuxkart",
        &["supertuxkart"],
    ),
    app(
        "0ad",
        "0 A.D.",
        "Historical real-time strategy",
        Play,
        "0ad",
        "0ad",
        &["0ad"],
    ),
    app(
        "wesnoth",
        "Battle for Wesnoth",
        "Turn-based fantasy strategy",
        Play,
        "wesnoth",
        "org.wesnoth.Wesnoth",
        &["wesnoth"],
    ),
    app(
        "openttd",
        "OpenTTD",
        "Build a transport empire",
        Play,
        "openttd",
        "openttd",
        &["openttd"],
    ),
    app(
        "luanti",
        "Luanti",
        "Build and explore voxel worlds",
        Play,
        "luanti",
        "org.luanti.luanti",
        &["luanti"],
    ),
    app(
        "retroarch",
        "RetroArch",
        "Play your classic games",
        Play,
        "retroarch",
        "retroarch",
        &["retroarch"],
    ),
    // Develop
    app(
        "vscode",
        "Code - OSS",
        "Code editor with extensions",
        Develop,
        "vscode",
        "code-oss",
        &["code"],
    ),
    app(
        "godot",
        "Godot Engine",
        "Make 2D and 3D games",
        Develop,
        "godot",
        "org.godotengine.Godot",
        &["godot"],
    ),
    app(
        "qtcreator",
        "Qt Creator",
        "IDE for Qt and C++",
        Develop,
        "QtProject-qtcreator",
        "org.qt-project.qtcreator",
        &["qtcreator"],
    ),
    app(
        "kate",
        "Kate",
        "Capable text editor",
        Develop,
        "kate",
        "org.kde.kate",
        &["kate"],
    ),
    app(
        "geany",
        "Geany",
        "Lightweight IDE",
        Develop,
        "geany",
        "geany",
        &["geany"],
    ),
    app(
        "meld",
        "Meld",
        "Compare files and folders",
        Develop,
        "org.gnome.Meld",
        "org.gnome.Meld",
        &["meld"],
    ),
    app(
        "dbeaver",
        "DBeaver",
        "Work with any database",
        Develop,
        "dbeaver",
        "dbeaver",
        &["dbeaver"],
    ),
    app(
        "emacs",
        "Emacs",
        "The extensible editor",
        Develop,
        "emacs",
        "emacs",
        &["emacs"],
    ),
    app(
        "gitg",
        "gitg",
        "Browse Git history",
        Develop,
        "org.gnome.gitg",
        "org.gnome.gitg",
        &["gitg"],
    ),
];

/// Discover: hero app and curated rows (title, ids).
pub const HERO: &str = "blender";
pub const ROWS: [(&str, &[&str]); 3] = [
    (
        "Essentials",
        &[
            "firefox",
            "libreoffice",
            "thunderbird",
            "vlc",
            "keepassxc",
            "signal",
        ],
    ),
    (
        "Make something",
        &["krita", "gimp", "inkscape", "kdenlive", "obs", "audacity"],
    ),
    (
        "Play tonight",
        &[
            "steam",
            "supertuxkart",
            "0ad",
            "wesnoth",
            "openttd",
            "luanti",
        ],
    ),
];

/// Page banner: an app, its pitch and a two-stop gradient from its own brand colours.
/// Both colours carry white text (accent contrast rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hero {
    pub app: &'static str,
    pub headline: &'static str,
    pub subhead: &'static str,
    /// Gradient start, end (`#rrggbb`).
    pub colors: [&'static str; 2],
}

const HEROES: [Hero; 5] = [
    Hero {
        app: HERO,
        headline: "Bring ideas to life in 3D",
        subhead: "Model, sculpt, animate and render. The free studio behind films and games.",
        colors: ["#b34a00", "#1f4e79"],
    },
    Hero {
        app: "krita",
        headline: "Paint without limits",
        subhead: "Brushes made by artists, for comics, concept art and illustration.",
        colors: ["#a3195b", "#1d4e9a"],
    },
    Hero {
        app: "thunderbird",
        headline: "All your mail in one place",
        subhead: "Accounts, calendars and contacts together, private by design.",
        colors: ["#0b5cc4", "#2a2f8f"],
    },
    Hero {
        app: "supertuxkart",
        headline: "Race your friends tonight",
        subhead: "Wild tracks, power-ups and split-screen fun for everyone.",
        colors: ["#a11a1a", "#3c2a80"],
    },
    Hero {
        app: "godot",
        headline: "Make the game you want to play",
        subhead: "A complete 2D and 3D engine. Free, open and tiny.",
        colors: ["#2c6493", "#1b3a57"],
    },
];

/// Hero for Discover (`None`) or a category page.
pub fn hero(page: Option<Category>) -> &'static Hero {
    let i = match page {
        None => 0,
        Some(Category::Create) => 1,
        Some(Category::Work) => 2,
        Some(Category::Play) => 3,
        Some(Category::Develop) => 4,
    };
    &HEROES[i]
}

pub fn get(id: &str) -> Option<&'static App> {
    CATALOG.iter().find(|a| a.id == id)
}

pub fn in_category(c: Category) -> impl Iterator<Item = &'static App> {
    CATALOG.iter().filter(move |a| a.category == c)
}

/// Case-insensitive match on name, summary and packages.
pub fn search(term: &str) -> Vec<&'static App> {
    let t = term.trim().to_lowercase();
    if t.is_empty() {
        return Vec::new();
    }
    CATALOG
        .iter()
        .filter(|a| {
            a.name.to_lowercase().contains(&t)
                || a.summary.to_lowercase().contains(&t)
                || a.packages.iter().any(|p| p.contains(&t))
        })
        .collect()
}

/// pacman package name: `[A-Za-z0-9][A-Za-z0-9._+-]*`, 1..=64.
pub fn is_pkg_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(c))
}

/// Fixed argv steps that install a catalog id or plain package name. None if neither.
pub fn install_plan(target: &str) -> Option<Vec<Vec<String>>> {
    let pkgs: Vec<&str> = match get(target) {
        Some(a) => a.packages.to_vec(),
        None if is_pkg_name(target) => vec![target],
        None => return None,
    };
    Some(vec![pacman("-S", "--needed", &pkgs)])
}

/// Fixed argv step that removes a catalog id or plain package name, with its unused dependencies.
pub fn remove_plan(target: &str) -> Option<Vec<Vec<String>>> {
    let pkgs: Vec<&str> = match get(target) {
        Some(a) => a.packages.to_vec(),
        None if is_pkg_name(target) => vec![target],
        None => return None,
    };
    Some(vec![pacman("-Rns", "", &pkgs)])
}

fn pacman(op: &str, extra: &str, pkgs: &[&str]) -> Vec<String> {
    let mut v = vec!["pacman".to_string(), op.into()];
    if !extra.is_empty() {
        v.push(extra.into());
    }
    v.push("--noconfirm".into());
    v.extend(pkgs.iter().map(|p| p.to_string()));
    v
}

/// Read-only repo search for words the catalog misses; None for empty or option-like terms.
pub fn search_argv(term: &str) -> Option<Vec<String>> {
    let t = term.trim();
    if t.is_empty() || t.starts_with('-') {
        return None;
    }
    Some(vec!["pacman".into(), "-Ss".into(), "--".into(), t.into()])
}

/// A `pacman -Ss` hit that is not in the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PkgHit {
    pub name: String,
    pub summary: String,
}

/// `pacman -Ss` output: "repo/name 1.2.3-1" then an indented description. Catalog packages dropped.
pub fn parse_pkg_search(text: &str) -> Vec<PkgHit> {
    let mut v: Vec<PkgHit> = Vec::new();
    let mut lines = text.lines();
    while let Some(l) = lines.next() {
        if l.starts_with(char::is_whitespace) {
            continue;
        }
        let summary = lines.next().unwrap_or("").trim();
        let Some(head) = l.split_whitespace().next() else {
            continue;
        };
        let name = head.rsplit_once('/').map_or(head, |(_, n)| n);
        if !is_pkg_name(name)
            || CATALOG.iter().any(|a| a.packages.contains(&name))
            || v.iter().any(|h| h.name == name)
        {
            continue;
        }
        v.push(PkgHit {
            name: name.into(),
            summary: summary.into(),
        });
    }
    v
}

/// Whether `<desktop>.desktop` is in any applications dir.
pub fn is_installed(app: &App, data_dirs: &[std::path::PathBuf]) -> bool {
    data_dirs.iter().any(|d| {
        d.join("applications")
            .join(format!("{}.desktop", app.desktop))
            .exists()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_a_readable_hero() {
        use shared::theme::{ACCENT_CONTRAST, Color};
        let pages: Vec<Option<Category>> = std::iter::once(None)
            .chain(Category::ALL.map(Some))
            .collect();
        let mut apps = Vec::new();
        for p in pages {
            let h = hero(p);
            let a = get(h.app).unwrap_or_else(|| panic!("{p:?} hero {} not in catalog", h.app));
            if let Some(c) = p {
                assert_eq!(a.category, c, "{} hero is from its own category", c.name());
            }
            assert!(!h.headline.is_empty() && h.subhead.len() <= 90, "{}", h.app);
            for hex in h.colors {
                let c = Color::parse(hex).expect(hex);
                assert!(
                    Color::contrast(Color::WHITE, c) >= ACCENT_CONTRAST,
                    "{} {hex}",
                    h.app
                );
            }
            apps.push(h.app);
        }
        assert_eq!(hero(None).app, HERO);
        apps.sort();
        apps.dedup();
        assert_eq!(apps.len(), 5, "one distinct hero per page");
    }

    #[test]
    fn catalog_is_curated_and_consistent() {
        assert_eq!(CATALOG.len(), 40);
        let mut ids: Vec<&str> = CATALOG.iter().map(|a| a.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 40, "unique ids");
        for a in CATALOG {
            assert!(
                !a.packages.is_empty() && a.packages.iter().all(|p| is_pkg_name(p)),
                "{}",
                a.id
            );
            assert!(
                !a.summary.ends_with('.') && a.summary.len() <= 40,
                "{}",
                a.id
            );
        }
        for c in Category::ALL {
            assert!(in_category(c).count() >= 8, "{c:?}");
        }
        assert!(get(HERO).is_some());
        for (_, row) in ROWS {
            assert!(row.iter().all(|id| get(id).is_some()));
        }
    }

    #[test]
    fn install_plans() {
        let p = |v: &[&str]| vec![v.iter().map(|x| x.to_string()).collect::<Vec<_>>()];
        assert_eq!(
            install_plan("gimp").unwrap(),
            p(&["pacman", "-S", "--needed", "--noconfirm", "gimp"])
        );
        assert_eq!(
            install_plan("steam").unwrap(),
            p(&["pacman", "-S", "--needed", "--noconfirm", "steam"])
        );
        assert_eq!(
            install_plan("htop").unwrap(),
            p(&["pacman", "-S", "--needed", "--noconfirm", "htop"])
        );
        assert_eq!(
            remove_plan("gimp").unwrap(),
            p(&["pacman", "-Rns", "--noconfirm", "gimp"])
        );
        for bad in ["", "-f", "a b", "x;rm", "../x"] {
            assert_eq!(install_plan(bad), None, "{bad}");
            assert_eq!(remove_plan(bad), None, "{bad}");
        }
    }

    #[test]
    fn search_runs_pacman_not_pkg() {
        let v = search_argv(" photo ").unwrap();
        assert_eq!(v, ["pacman", "-Ss", "--", "photo"]);
        assert_eq!(search_argv("  "), None);
        assert_eq!(search_argv("-Syu"), None);
    }

    #[test]
    fn search_and_pkg_hits() {
        assert_eq!(search("PHOTO").first().map(|a| a.id), Some("gimp"));
        assert!(search("  ").is_empty());
        let t = "extra/gimp 3.0.4-1\n    GNU Image Manipulation Program\nextra/htop 3.3.0-1\n    Better top(1)\nextra/htop 3.3.0-1\n    dup\n";
        assert_eq!(
            parse_pkg_search(t),
            vec![PkgHit {
                name: "htop".into(),
                summary: "Better top(1)".into()
            }]
        );
    }

    #[test]
    fn installed_from_desktop_files() {
        let d = std::env::temp_dir().join(format!("wol-store-{}", std::process::id()));
        std::fs::create_dir_all(d.join("applications")).unwrap();
        std::fs::write(d.join("applications/firefox.desktop"), "").unwrap();
        assert!(is_installed(
            get("firefox").unwrap(),
            std::slice::from_ref(&d)
        ));
        assert!(!is_installed(
            get("gimp").unwrap(),
            std::slice::from_ref(&d)
        ));
        std::fs::remove_dir_all(d).unwrap();
    }
}
