// SPDX-License-Identifier: BSD-2-Clause
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

/// Languages and toolchains as they are packaged: name and one line, each a real package.
pub const TOOLCHAINS: &[(&str, &str)] = &[
    ("gcc", "GNU compiler for C and C++"),
    ("clang", "LLVM compiler for C and C++"),
    ("rust", "Rust compiler and Cargo"),
    ("go", "Go compiler and tools"),
    ("zig", "Zig compiler and toolchain"),
    ("crystal", "Crystal compiler"),
    ("dotnet-sdk", ".NET SDK"),
    ("nodejs", "JavaScript runtime"),
    ("python", "Python language"),
    ("ruby", "Ruby language"),
    ("lua", "Lua language"),
    ("jdk-openjdk", "Java development kit"),
    ("kotlin", "Kotlin compiler"),
    ("dart", "Dart SDK"),
    ("ghc", "Haskell compiler"),
    ("cmake", "Build system generator"),
    ("meson", "Build system"),
    ("hvm", "Parallel runtime for functional programs"),
    ("zenc", "Zen C compiler"),
    ("bend", "Massively parallel programming language"),
];

/// Where a kit's parts come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KitSource {
    /// System packages: one wol-admind `pkg-install` (one approval) for every part.
    Packages,
    /// A wopkg recipe installed for this account only (no approval, no system change).
    Wopkg(&'static str),
}

/// One thing a kit brings, in the order the kit page lists it (most wanted first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KitPart {
    pub name: &'static str,
    /// What it lets people do, one line, sentence case, no full stop.
    pub why: &'static str,
    /// Packages for this part (empty for a wopkg kit).
    pub packages: &'static [&'static str],
}

/// A Store kit (ADR 0032 B, the "Adobe kit" pattern): one tile, one click, one approval for a
/// set of parts. Kits are never installed by default; the person asks for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kit {
    /// Store target: wol-admind `pkg-install <id>` resolves it (never a pacman name).
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    /// Full-colour tile icon (`design/icons/apps/<icon>.svg`, a `wolos-kit-*` pixel tile).
    pub icon: &'static str,
    pub source: KitSource,
    pub parts: &'static [KitPart],
    /// One sentence under the parts (licences, what stays separate).
    pub note: &'static str,
}

const fn part(name: &'static str, why: &'static str, packages: &'static [&'static str]) -> KitPart {
    KitPart {
        name,
        why,
        packages,
    }
}

/// The Kits shelf, in order. The codecs kit lists its parts by how often people need them.
pub const KITS: [Kit; 2] = [
    Kit {
        id: "codecs-kit",
        name: "Media Codecs",
        summary: "Play and make MP3, AAC, H.264, HEVC and more",
        icon: "wolos-kit-codecs",
        source: KitSource::Packages,
        parts: &[
            part(
                "MP3",
                "Music files and podcasts",
                &["gst-plugins-good", "lame"],
            ),
            part(
                "AAC",
                "Music from stores, phone videos and streams",
                &["gst-libav", "faad2"],
            ),
            part(
                "H.264",
                "Most videos on the web and from cameras",
                &["gst-libav", "gst-plugins-ugly", "x264"],
            ),
            part(
                "HEVC (H.265)",
                "Videos from newer phones and 4K files",
                &["gst-libav", "x265", "libde265"],
            ),
            part(
                "AC3 (Dolby Digital)",
                "Sound on DVDs and many films",
                &["gst-libav", "a52dec"],
            ),
            part(
                "Fonts",
                "Documents and web pages look as their authors meant",
                &[
                    "ttf-liberation",
                    "ttf-dejavu",
                    "noto-fonts",
                    "noto-fonts-cjk",
                    "noto-fonts-emoji",
                ],
            ),
        ],
        note: "Some of these formats are patented in some countries. You choose whether to add them.",
    },
    Kit {
        id: "craft-suite",
        name: "Craft Suite",
        summary: "Creative apps from one self-updating launcher",
        icon: "wolos-kit-craft-suite",
        source: KitSource::Wopkg("craft-launcher"),
        parts: &[part(
            "Craft Launcher",
            "Installs and updates the Craft apps for you",
            &[],
        )],
        note: "Installed for your account only and checked before it runs. It updates itself.",
    },
];

pub fn kit(id: &str) -> Option<&'static Kit> {
    KITS.iter().find(|k| k.id == id)
}

impl Kit {
    /// Every package of every part, first mention first, without duplicates.
    pub fn packages(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Vec::new();
        for p in self.parts {
            for n in p.packages {
                if !v.contains(n) {
                    v.push(n);
                }
            }
        }
        v
    }

    /// `wopkg` arguments for a wopkg kit (`recipe install <recipe>`), None for a packages kit.
    /// Straight to the recipe, never through the search index: an index built before the recipe
    /// shipped resolves the name to a look-alike (the AUR's craft-launcher-bin), and `-y` never
    /// answers for a community package, so the click did nothing and still exited 0.
    pub fn wopkg_args(&self) -> Option<Vec<String>> {
        match self.source {
            KitSource::Wopkg(r) => Some(vec!["recipe".into(), "install".into(), r.into()]),
            KitSource::Packages => None,
        }
    }

    /// Whether the kit can be installed here: a packages kit always (pacman resolves it); a
    /// wopkg kit only when its recipe is in one of `recipe_dirs` (`<dir>/<recipe>/recipe.toml`).
    /// A kit that cannot apply is not shown (STANDARDS: applies or not shown).
    pub fn available(&self, recipe_dirs: &[std::path::PathBuf]) -> bool {
        match self.source {
            KitSource::Packages => true,
            KitSource::Wopkg(r) => recipe_dirs
                .iter()
                .any(|d| d.join(r).join("recipe.toml").is_file()),
        }
    }
}

/// Where wopkg finds official recipes (as `wopkg/src/wopkg/recipe.cr`).
pub const RECIPES_DIR: &str = "/usr/share/wopkg/recipes";

/// Recipe dirs: `$WOPKG_RECIPES` (colon-separated) first, then [`RECIPES_DIR`].
pub fn recipe_dirs(env: Option<&str>) -> Vec<std::path::PathBuf> {
    env.unwrap_or("")
        .split(':')
        .filter(|d| !d.is_empty())
        .chain([RECIPES_DIR])
        .map(std::path::PathBuf::from)
        .collect()
}

/// The Kits shelf for this system, in [`KITS`] order.
pub fn shown_kits(recipe_dirs: &[std::path::PathBuf]) -> Vec<&'static Kit> {
    KITS.iter().filter(|k| k.available(recipe_dirs)).collect()
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
    let pkgs: Vec<&str> = match (get(target), kit(target)) {
        (Some(a), _) => a.packages.to_vec(),
        // A wopkg kit is a user install, never a wol-admind step.
        (_, Some(k)) if k.source == KitSource::Packages => k.packages(),
        (_, Some(_)) => return None,
        (None, None) if is_pkg_name(target) => vec![target],
        (None, None) => return None,
    };
    // `--noprogressbar`: plain per-package lines the App Store turns into a percentage
    // (`crate::progress`).
    Some(vec![pacman("-S", &["--needed", "--noprogressbar"], &pkgs)])
}

/// `pacman -Si` in C locale for the details page; None for an invalid name.
pub fn info_argv(pkg: &str) -> Option<Vec<String>> {
    is_pkg_name(pkg).then(|| {
        ["env", "LC_ALL=C", "pacman", "-Si", "--", pkg]
            .map(String::from)
            .to_vec()
    })
}

/// "Installed Size  : 154.93 MiB" -> "154.93 MiB".
pub fn parse_installed_size(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == "Installed Size").then(|| v.trim().to_owned())
    })
}

/// Fixed argv step that removes a catalog id or plain package name, with its unused dependencies.
pub fn remove_plan(target: &str) -> Option<Vec<Vec<String>>> {
    // A kit's packages are shared with other apps (gst-libav, fonts): it has no one-click remove.
    if kit(target).is_some() {
        return None;
    }
    let pkgs: Vec<&str> = match get(target) {
        Some(a) => a.packages.to_vec(),
        None if is_pkg_name(target) => vec![target],
        None => return None,
    };
    Some(vec![pacman("-Rns", &[], &pkgs)])
}

/// The sealed-system pacman (`wol pkg --`, `controller::pkg`: a live layer, the pending slot for
/// kernel and init packages), as wol-admind's `PkgInstallFile` runs it. Not the bare `pacman`:
/// wol-admind's children get `PATH=/usr/bin:/usr/sbin` (`privileged::PATH`), so that was the stock
/// pacman, whose database lives on the read-only /usr ("unable to lock database"; MANUAL 0.12.0:
/// Settings > Modules > Install failed right after the password).
fn pacman(op: &str, extra: &[&str], pkgs: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = ["wol", "pkg", "--", op].map(String::from).to_vec();
    v.extend(extra.iter().map(|e| e.to_string()));
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
    Some(vec![
        "pacman".into(),
        "-Ss".into(),
        "--".into(),
        literal_regex(t),
    ])
}

/// `t` as a POSIX extended regular expression that matches it literally: pacman's `-Ss` term is
/// a regex, so "(" or "vlc[" failed with "invalid regular expression" (a silent "No results")
/// and "c++" only worked by chance (QA-STORE-39, QA-STORE-79).
pub fn literal_regex(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
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
    fn kits_are_one_click_and_never_default() {
        let ids: Vec<&str> = KITS.iter().map(|k| k.id).collect();
        assert_eq!(ids, ["codecs-kit", "craft-suite"]);
        for k in KITS {
            assert!(get(k.id).is_none(), "{} is not a catalog id", k.id);
            assert!(!k.parts.is_empty() && k.summary.len() <= 50, "{}", k.id);
            assert!(
                !k.summary.ends_with('.') && k.note.ends_with('.'),
                "{}",
                k.id
            );
            assert!(k.packages().iter().all(|p| is_pkg_name(p)), "{}", k.id);
            assert_eq!(remove_plan(k.id), None, "{} has no one-click remove", k.id);
            // Kits are not apps: no Discover row and no category lists them.
            assert!(ROWS.iter().all(|(_, r)| !r.contains(&k.id)));
        }
        assert_eq!(kit("nope"), None);
    }

    #[test]
    fn the_codecs_kit_lists_what_people_need_most_first() {
        let k = kit("codecs-kit").unwrap();
        let names: Vec<&str> = k.parts.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            [
                "MP3",
                "AAC",
                "H.264",
                "HEVC (H.265)",
                "AC3 (Dolby Digital)",
                "Fonts"
            ]
        );
        assert!(k.parts.iter().all(|p| !p.packages.is_empty()));
        let pkgs = k.packages();
        let mut d = pkgs.clone();
        d.sort();
        d.dedup();
        assert_eq!(d.len(), pkgs.len(), "each package once");
        assert_eq!(pkgs[..2], ["gst-plugins-good", "lame"]);
        // One wol-admind step, one approval, every package.
        let plan = install_plan("codecs-kit").unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(
            plan[0][..7],
            [
                "wol",
                "pkg",
                "--",
                "-S",
                "--needed",
                "--noprogressbar",
                "--noconfirm"
            ]
        );
        assert_eq!(
            plan[0][7..],
            pkgs.iter().map(|p| p.to_string()).collect::<Vec<_>>()[..]
        );
        assert_eq!(k.wopkg_args(), None);
    }

    #[test]
    fn craft_suite_is_a_user_install_through_wopkg() {
        let k = kit("craft-suite").unwrap();
        assert_eq!(k.source, KitSource::Wopkg("craft-launcher"));
        assert_eq!(
            k.wopkg_args().unwrap(),
            [
                "recipe".to_string(),
                "install".into(),
                "craft-launcher".into()
            ]
        );
        assert!(k.packages().is_empty());
        // wol-admind never installs it: no system step for a user kit.
        assert_eq!(install_plan("craft-suite"), None);
    }

    #[test]
    fn craft_suite_shows_only_when_its_recipe_exists() {
        let d = std::env::temp_dir().join(format!("wol-recipes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let ids = |dirs: &[std::path::PathBuf]| -> Vec<&str> {
            shown_kits(dirs).iter().map(|k| k.id).collect()
        };
        let dirs = vec![d.clone()];
        assert_eq!(ids(&dirs), ["codecs-kit"], "no recipe, no tile");
        std::fs::create_dir_all(d.join("craft-launcher")).unwrap();
        assert_eq!(
            ids(&dirs),
            ["codecs-kit"],
            "a folder without recipe.toml is not a recipe"
        );
        std::fs::write(
            d.join("craft-launcher/recipe.toml"),
            "name = \"craft-launcher\"\n",
        )
        .unwrap();
        assert_eq!(ids(&dirs), ["codecs-kit", "craft-suite"]);
        assert!(kit("codecs-kit").unwrap().available(&[]));
        std::fs::remove_dir_all(d).unwrap();
        assert_eq!(
            recipe_dirs(Some("/a::/b")),
            ["/a", "/b", RECIPES_DIR].map(std::path::PathBuf::from)
        );
        assert_eq!(recipe_dirs(None), [std::path::PathBuf::from(RECIPES_DIR)]);
    }

    #[test]
    fn the_shipped_recipes_show_the_craft_suite() {
        // wopkg/recipes is what the wopkg package installs in /usr/share/wopkg/recipes.
        let shipped = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wopkg/recipes");
        let ids: Vec<&str> = shown_kits(&[shipped.clone()])
            .iter()
            .map(|k| k.id)
            .collect();
        assert_eq!(ids, ["codecs-kit", "craft-suite"]);
        let t = std::fs::read_to_string(shipped.join("craft-launcher/recipe.toml")).unwrap();
        assert!(
            t.contains("name = \"craft-launcher\""),
            "the recipe is the kit's"
        );
    }

    #[test]
    fn package_info_for_the_details_page() {
        assert_eq!(
            info_argv("gimp").unwrap().join(" "),
            "env LC_ALL=C pacman -Si -- gimp"
        );
        assert_eq!(info_argv("-x"), None);
        let t = "Name            : gimp\nInstalled Size  : 154.93 MiB\nPackager        : X\n";
        assert_eq!(parse_installed_size(t).as_deref(), Some("154.93 MiB"));
    }

    #[test]
    fn install_plans() {
        let p = |v: &[&str]| vec![v.iter().map(|x| x.to_string()).collect::<Vec<_>>()];
        assert_eq!(
            install_plan("gimp").unwrap(),
            p(&[
                "wol",
                "pkg",
                "--",
                "-S",
                "--needed",
                "--noprogressbar",
                "--noconfirm",
                "gimp"
            ])
        );
        assert_eq!(
            install_plan("steam").unwrap(),
            p(&[
                "wol",
                "pkg",
                "--",
                "-S",
                "--needed",
                "--noprogressbar",
                "--noconfirm",
                "steam"
            ])
        );
        assert_eq!(
            install_plan("htop").unwrap(),
            p(&[
                "wol",
                "pkg",
                "--",
                "-S",
                "--needed",
                "--noprogressbar",
                "--noconfirm",
                "htop"
            ])
        );
        assert_eq!(
            remove_plan("gimp").unwrap(),
            p(&["wol", "pkg", "--", "-Rns", "--noconfirm", "gimp"])
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
        // QA-STORE-39/79: the term is matched literally, never as pacman's regex.
        assert_eq!(search_argv("(").unwrap()[3], "\\(");
        assert_eq!(search_argv("vlc[").unwrap()[3], "vlc\\[");
        assert_eq!(search_argv("c++").unwrap()[3], "c\\+\\+");
        assert_eq!(search_argv("gimp").unwrap()[3], "gimp");
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
