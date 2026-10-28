//! Flatpak apps next to native ones: the consent helper's arguments, the grants it records
//! (`{"grants":[{"app":"<install dir>/app/<id>","capability":"flatpak:<perm>"}]}`), the words
//! Settings and the consent sheet show, and the `flatpak` argv that turns one permission off or on
//! (`flatpak override`). Kept apart from [`crate::consent`], whose names are executable paths.
use serde::Deserialize;

/// Environment variable the Flatpak build reads to find the consent helper.
pub const HELPER_ENV: &str = "FLATPAK_WOLOS_CONSENT_HELPER";
/// Where the image installs the helper.
pub const HELPER: &str = "/usr/bin/wol-flatpak-consent";
pub const SYSTEM_GRANTS: &str = "/etc/wolos/flatpak-consent.json";
/// Below `$HOME`.
pub const USER_GRANTS: &str = ".config/wolos/flatpak-consent.json";
pub const SYSTEM_DIR: &str = "/var/lib/flatpak";
/// Below `$HOME`.
pub const USER_DIR: &str = ".local/share/flatpak";
pub const REMOTE: &str = "flathub";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    User,
    System,
}

impl Scope {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "system" => Some(Self::System),
            _ => None,
        }
    }
    pub fn flag(self) -> &'static str {
        match self {
            Self::User => "--user",
            Self::System => "--system",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Sockets,
    Filesystems,
    Devices,
    Shared,
    Features,
    SessionBus,
    SystemBus,
}

/// One permission as the helper passes it: `sockets=x11`, `session-bus:org.foo=talk`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Perm {
    pub key: Key,
    /// Socket, path, device, bus name...
    pub value: String,
    /// Bus policy (`talk`, `own`, `see`); empty for the rest.
    pub policy: String,
}

fn safe(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && !s.starts_with('-')
        && !s.chars().any(|c| c.is_control() || c.is_whitespace())
}

impl Perm {
    pub fn parse(s: &str) -> Option<Self> {
        let bus = |key, rest: &str| {
            let (name, policy) = rest.rsplit_once('=')?;
            (safe(name) && matches!(policy, "talk" | "own" | "see")).then(|| Perm {
                key,
                value: name.into(),
                policy: policy.into(),
            })
        };
        if let Some(r) = s.strip_prefix("session-bus:") {
            return bus(Key::SessionBus, r);
        }
        if let Some(r) = s.strip_prefix("system-bus:") {
            return bus(Key::SystemBus, r);
        }
        let (k, v) = s.split_once('=')?;
        let key = match k {
            "sockets" => Key::Sockets,
            "filesystems" => Key::Filesystems,
            "devices" => Key::Devices,
            "shared" => Key::Shared,
            "features" => Key::Features,
            _ => return None,
        };
        safe(v).then(|| Perm {
            key,
            value: v.into(),
            policy: String::new(),
        })
    }

    /// Back to the helper's spelling (the grant's capability without `flatpak:`).
    pub fn spelling(&self) -> String {
        match self.key {
            Key::SessionBus => format!("session-bus:{}={}", self.value, self.policy),
            Key::SystemBus => format!("system-bus:{}={}", self.value, self.policy),
            k => format!(
                "{}={}",
                match k {
                    Key::Sockets => "sockets",
                    Key::Filesystems => "filesystems",
                    Key::Devices => "devices",
                    Key::Shared => "shared",
                    _ => "features",
                },
                self.value
            ),
        }
    }

    /// What it lets the app do, in plain words.
    pub fn plain(&self) -> String {
        let v = self.value.as_str();
        let ro = v.ends_with(":ro");
        let path = v
            .trim_end_matches(":ro")
            .trim_end_matches(":rw")
            .trim_end_matches(":create");
        let see = if ro { "See" } else { "Use" };
        match self.key {
            Key::Filesystems => match path {
                "home" | "~" => format!("{see} your Home folder"),
                "host" => format!("{see} all your files"),
                "host-os" => format!("{see} system files"),
                "host-etc" => format!("{see} system settings files"),
                p if p.starts_with("xdg-") => {
                    let name = p.trim_start_matches("xdg-").split('/').next().unwrap_or(p);
                    let name = match name {
                        "download" => "Downloads",
                        "documents" => "Documents",
                        "music" => "Music",
                        "pictures" => "Pictures",
                        "videos" => "Movies",
                        "desktop" => "Desktop",
                        "config" => "app settings",
                        "run" => "running services",
                        other => other,
                    };
                    format!("{see} your {name} folder")
                }
                p => format!("{see} the folder {p}"),
            },
            Key::Sockets => match v {
                "x11" | "fallback-x11" => "Show windows the older way (X11)".into(),
                "wayland" => "Show windows".into(),
                "pulseaudio" => "Play and record sound".into(),
                "cups" => "Print".into(),
                "ssh-auth" => "Use your SSH keys".into(),
                "gpg-agent" => "Use your GPG keys".into(),
                "pcsc" => "Use smart cards".into(),
                "session-bus" => "Talk to every app in your session".into(),
                "system-bus" => "Talk to every system service".into(),
                other => format!("Use {other}"),
            },
            Key::Devices => match v {
                "dri" => "Use the graphics card".into(),
                "all" => "Use all devices, including cameras".into(),
                "kvm" => "Run virtual machines".into(),
                "shm" => "Share memory with other apps".into(),
                "input" => "Read game controllers and input devices".into(),
                other => format!("Use the device {other}"),
            },
            Key::Shared => match v {
                "network" => "Use the network".into(),
                "ipc" => "Share data with other apps".into(),
                other => format!("Share {other}"),
            },
            Key::Features => match v {
                "bluetooth" => "Use Bluetooth".into(),
                "devel" => "Debug other programs".into(),
                "multiarch" => "Run 32-bit programs".into(),
                "canbus" => "Use CAN bus devices".into(),
                "per-app-dev-shm" => "Share memory with its own helpers".into(),
                other => format!("Use {other}"),
            },
            Key::SessionBus | Key::SystemBus => {
                let what = if self.key == Key::SessionBus {
                    "the app"
                } else {
                    "the system service"
                };
                if self.policy == "own" {
                    format!("Act as {what} {v}")
                } else {
                    format!("Talk to {what} {v}")
                }
            }
        }
    }

    /// `flatpak override <scope> <flag> <id>` turning it on (`allow`) or off.
    pub fn override_argv(&self, scope: Scope, app_id: &str, allow: bool) -> Vec<String> {
        let flag = |on: &str, off: &str| format!("{}={}", if allow { on } else { off }, self.value);
        let f = match self.key {
            Key::Filesystems => flag("--filesystem", "--nofilesystem"),
            Key::Sockets => flag("--socket", "--nosocket"),
            Key::Devices => flag("--device", "--nodevice"),
            Key::Shared => flag("--share", "--unshare"),
            Key::Features => flag("--allow", "--disallow"),
            Key::SessionBus if !allow => flag("", "--no-talk-name"),
            Key::SessionBus if self.policy == "own" => flag("--own-name", ""),
            Key::SessionBus => flag("--talk-name", ""),
            Key::SystemBus if allow => flag("--system-talk-name", ""),
            Key::SystemBus => flag("", "--system-no-talk-name"),
        };
        ["flatpak", "override", scope.flag(), &f, app_id]
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    }

    /// Turned off in a `flatpak override` file (`[Context] sockets=!x11;`, bus `name=none`).
    pub fn denied_in(&self, overrides: &str) -> bool {
        let (section, key) = match self.key {
            Key::SessionBus => ("[Session Bus Policy]", None),
            Key::SystemBus => ("[System Bus Policy]", None),
            Key::Sockets => ("[Context]", Some("sockets")),
            Key::Filesystems => ("[Context]", Some("filesystems")),
            Key::Devices => ("[Context]", Some("devices")),
            Key::Shared => ("[Context]", Some("shared")),
            Key::Features => ("[Context]", Some("features")),
        };
        let mut inside = false;
        for line in overrides.lines().map(str::trim) {
            if line.starts_with('[') {
                inside = line == section;
                continue;
            }
            let Some((k, v)) = line.split_once('=').filter(|_| inside) else {
                continue;
            };
            match key {
                Some(key) if k == key => {
                    let off = format!("!{}", self.value);
                    if v.split(';').any(|x| x == off) {
                        return true;
                    }
                }
                None if k == self.value => return v == "none",
                _ => {}
            }
        }
        false
    }
}

/// `flatpak override <scope> --reset <id>`.
pub fn reset_argv(scope: Scope, app_id: &str) -> Vec<String> {
    ["flatpak", "override", scope.flag(), "--reset", app_id]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
}

/// Reverse-DNS app id: at least three dot-separated parts of `[A-Za-z0-9_-]`, first not a digit.
pub fn is_app_id(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    s.len() <= 255
        && parts.len() >= 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && !p.starts_with(|c: char| c.is_ascii_digit() || c == '-')
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

/// What the Flatpak build asked: `install|update <app-id> user|system -- <perm>...`.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub update: bool,
    pub app_id: String,
    pub scope: Scope,
    pub perms: Vec<Perm>,
}

impl Request {
    /// None for anything else (the helper then denies). Permissions it cannot read are dropped.
    pub fn parse(args: &[String]) -> Option<Self> {
        let [verb, id, scope, sep, rest @ ..] = args else {
            return None;
        };
        let update = match verb.as_str() {
            "install" => false,
            "update" => true,
            _ => return None,
        };
        (is_app_id(id) && sep == "--").then_some(())?;
        Some(Self {
            update,
            app_id: id.clone(),
            scope: Scope::parse(scope)?,
            perms: rest.iter().filter_map(|p| Perm::parse(p)).collect(),
        })
    }
}

/// One recorded permission of one Flatpak app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub app_id: String,
    pub scope: Scope,
    pub perm: Perm,
}

/// Grants from one consent file; entries that are not `.../app/<id>` + `flatpak:<perm>` are skipped.
pub fn parse_grants(json: &str, scope: Scope) -> Vec<Grant> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        grants: Vec<G>,
    }
    #[derive(Deserialize)]
    struct G {
        app: String,
        capability: String,
    }
    let Ok(f) = serde_json::from_str::<File>(json) else {
        return Vec::new();
    };
    let mut out: Vec<Grant> = Vec::new();
    for g in f.grants {
        let id = g.app.rsplit_once("/app/").map(|(_, id)| id);
        let perm = g.capability.strip_prefix("flatpak:").and_then(Perm::parse);
        if let (Some(id), Some(perm)) = (id.filter(|i| is_app_id(i)), perm) {
            let g = Grant {
                app_id: id.into(),
                scope,
                perm,
            };
            if !out.contains(&g) {
                out.push(g);
            }
        }
    }
    out
}

/// `Name=` of the `[Desktop Entry]` group (the app's exported desktop file).
pub fn desktop_name(text: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
        } else if inside && let Some(n) = line.strip_prefix("Name=") {
            return Some(n.trim().to_owned()).filter(|n| !n.is_empty());
        }
    }
    None
}

/// Where an installed app's exported desktop file and icon live, below the install dir.
pub fn export_paths(install_dir: &str, app_id: &str) -> (String, String) {
    let base = format!("{install_dir}/app/{app_id}/current/active/export/share");
    (
        format!("{base}/applications/{app_id}.desktop"),
        format!("{base}/icons/hicolor/128x128/apps/{app_id}.png"),
    )
}

/// One Flatpak app with recorded permissions, as Settings > Privacy lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub scope: Scope,
    pub name: String,
    /// Exported icon file, or the app id as an icon name.
    pub icon: String,
    /// Each permission and whether it is on now (not turned off by an override).
    pub perms: Vec<(Perm, bool)>,
}

/// Grants from both consent files, grouped per app, with the override files' verdicts.
pub fn apps(h: &dyn crate::system::Host, home: &str) -> Vec<App> {
    let mut out: Vec<App> = Vec::new();
    for (scope, file, dir) in [
        (
            Scope::User,
            format!("{home}/{USER_GRANTS}"),
            format!("{home}/{USER_DIR}"),
        ),
        (
            Scope::System,
            SYSTEM_GRANTS.to_owned(),
            SYSTEM_DIR.to_owned(),
        ),
    ] {
        let grants = h
            .read_text(&file)
            .map(|t| parse_grants(&t, scope))
            .unwrap_or_default();
        for g in grants {
            let i = match out
                .iter()
                .position(|a| a.id == g.app_id && a.scope == scope)
            {
                Some(i) => i,
                None => {
                    let (desktop, icon) = export_paths(&dir, &g.app_id);
                    let name = h
                        .read_text(&desktop)
                        .and_then(|t| desktop_name(&t))
                        .unwrap_or_else(|| g.app_id.rsplit('.').next().unwrap_or("").to_owned());
                    let icon = if std::path::Path::new(&icon).is_file() {
                        icon
                    } else {
                        g.app_id.clone()
                    };
                    out.push(App {
                        id: g.app_id.clone(),
                        scope,
                        name,
                        icon,
                        perms: Vec::new(),
                    });
                    out.len() - 1
                }
            };
            let overrides = h
                .read_text(&format!("{dir}/overrides/{}", g.app_id))
                .unwrap_or_default();
            let on = !g.perm.denied_in(&overrides);
            out[i].perms.push((g.perm, on));
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

// ---- App Store: Flathub as a second source ----

/// A Flathub app found by `flatpak search`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    pub name: String,
    pub summary: String,
}

fn argv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

/// Read-only Flathub search (the local appstream); None for empty or option-like terms.
pub fn search_argv(term: &str) -> Option<Vec<String>> {
    let t = term.trim();
    (!t.is_empty() && !t.starts_with('-') && !t.chars().any(char::is_control)).then(|| {
        argv(&[
            "flatpak",
            "search",
            "--columns=application,name,description,remotes",
            "--",
            t,
        ])
    })
}

/// `flatpak search` rows (tab-separated) from Flathub. `native(hit)` says a native package
/// already covers it (ours, then Cachy, then Arch win): those are dropped, one tile per app.
pub fn parse_search(text: &str, native: impl Fn(&Hit) -> bool) -> Vec<Hit> {
    let mut v: Vec<Hit> = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let [id, name, summary, remotes, ..] = f[..] else {
            continue;
        };
        if !is_app_id(id)
            || !remotes.split(',').any(|r| r.trim() == REMOTE)
            || v.iter().any(|h| h.id == id)
        {
            continue;
        }
        let h = Hit {
            id: id.into(),
            name: name.trim().into(),
            summary: summary.trim().into(),
        };
        if !native(&h) {
            v.push(h);
        }
    }
    v
}

/// The package name a native build of `hit` would most likely carry (`org.gimp.GIMP` -> `gimp`).
pub fn native_guess(hit: &Hit) -> String {
    hit.id.rsplit('.').next().unwrap_or("").to_lowercase()
}

/// Install for this account only: no administrator password, the consent sheet still asks.
pub fn install_argv(app_id: &str) -> Vec<String> {
    argv(&[
        "flatpak",
        "install",
        "--user",
        "--noninteractive",
        "-y",
        REMOTE,
        app_id,
    ])
}

pub fn remove_argv(app_id: &str) -> Vec<String> {
    argv(&[
        "flatpak",
        "uninstall",
        "--user",
        "--noninteractive",
        "-y",
        app_id,
    ])
}

/// Software Update: every Flatpak app and runtime of this account.
pub fn update_argv() -> Vec<String> {
    argv(&["flatpak", "update", "--user", "--noninteractive", "-y"])
}

/// `flatpak list --app --columns=application` (both scopes).
pub fn installed_argv() -> Vec<String> {
    argv(&["flatpak", "list", "--app", "--columns=application"])
}

/// `flatpak remote-ls --updates --app --columns=application`: ids with an update waiting.
pub fn updates_argv() -> Vec<String> {
    argv(&[
        "flatpak",
        "remote-ls",
        "--updates",
        "--app",
        "--columns=application",
    ])
}

/// One app id per line (list, remote-ls); anything else skipped.
pub fn parse_ids(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| is_app_id(l))
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Perm {
        Perm::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    #[test]
    fn permissions_read_in_plain_words() {
        for (s, want) in [
            ("filesystems=home", "Use your Home folder"),
            ("filesystems=xdg-download:ro", "See your Downloads folder"),
            ("filesystems=host", "Use all your files"),
            ("shared=network", "Use the network"),
            ("sockets=x11", "Show windows the older way (X11)"),
            ("sockets=pulseaudio", "Play and record sound"),
            ("devices=dri", "Use the graphics card"),
            ("devices=all", "Use all devices, including cameras"),
            ("features=bluetooth", "Use Bluetooth"),
            (
                "session-bus:org.freedesktop.Notifications=talk",
                "Talk to the app org.freedesktop.Notifications",
            ),
            (
                "system-bus:org.freedesktop.UPower=talk",
                "Talk to the system service org.freedesktop.UPower",
            ),
        ] {
            assert_eq!(p(s).plain(), want, "{s}");
            assert_eq!(p(s).spelling(), s);
        }
    }

    #[test]
    fn bad_permissions_are_refused() {
        for s in [
            "",
            "sockets=",
            "sockets=--user",
            "sockets=x 11",
            "bogus=x",
            "session-bus:org.foo=rule",
            "filesystems=home\n",
        ] {
            assert_eq!(Perm::parse(s), None, "{s:?}");
        }
    }

    #[test]
    fn override_flags_per_kind() {
        let show =
            |s: &str, scope, allow| p(s).override_argv(scope, "org.gimp.GIMP", allow).join(" ");
        for (s, allow, want) in [
            ("filesystems=home", false, "--nofilesystem=home"),
            ("filesystems=home", true, "--filesystem=home"),
            ("sockets=x11", false, "--nosocket=x11"),
            ("sockets=x11", true, "--socket=x11"),
            ("devices=dri", false, "--nodevice=dri"),
            ("devices=dri", true, "--device=dri"),
            ("shared=network", false, "--unshare=network"),
            ("shared=network", true, "--share=network"),
            ("features=bluetooth", false, "--disallow=bluetooth"),
            ("features=bluetooth", true, "--allow=bluetooth"),
            (
                "session-bus:org.foo.Bar=talk",
                false,
                "--no-talk-name=org.foo.Bar",
            ),
            (
                "session-bus:org.foo.Bar=talk",
                true,
                "--talk-name=org.foo.Bar",
            ),
            (
                "session-bus:org.foo.Bar=own",
                true,
                "--own-name=org.foo.Bar",
            ),
            (
                "system-bus:org.foo.Bar=talk",
                false,
                "--system-no-talk-name=org.foo.Bar",
            ),
            (
                "system-bus:org.foo.Bar=talk",
                true,
                "--system-talk-name=org.foo.Bar",
            ),
        ] {
            assert_eq!(
                show(s, Scope::User, allow),
                format!("flatpak override --user {want} org.gimp.GIMP"),
                "{s} {allow}"
            );
        }
        assert_eq!(
            show("shared=network", Scope::System, false),
            "flatpak override --system --unshare=network org.gimp.GIMP"
        );
        assert_eq!(
            reset_argv(Scope::User, "org.gimp.GIMP").join(" "),
            "flatpak override --user --reset org.gimp.GIMP"
        );
    }

    #[test]
    fn overrides_file_says_what_is_off() {
        let o = "[Context]\nshared=!network;\nsockets=!x11;wayland;\n\n[Session Bus Policy]\norg.foo.Bar=none\norg.ok.App=talk\n";
        assert!(p("shared=network").denied_in(o));
        assert!(p("sockets=x11").denied_in(o));
        assert!(!p("sockets=wayland").denied_in(o));
        assert!(!p("filesystems=home").denied_in(o));
        assert!(p("session-bus:org.foo.Bar=talk").denied_in(o));
        assert!(!p("session-bus:org.ok.App=talk").denied_in(o));
        assert!(!p("system-bus:org.foo.Bar=talk").denied_in(o));
        assert!(!p("shared=network").denied_in(""));
    }

    #[test]
    fn helper_arguments() {
        let a = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let r = Request::parse(&a(&[
            "install",
            "org.gimp.GIMP",
            "user",
            "--",
            "filesystems=home",
            "shared=network",
            "nonsense",
        ]))
        .unwrap();
        assert!(!r.update && r.scope == Scope::User && r.app_id == "org.gimp.GIMP");
        assert_eq!(r.perms, [p("filesystems=home"), p("shared=network")]);
        assert!(
            Request::parse(&a(&["update", "org.gimp.GIMP", "system", "--"]))
                .unwrap()
                .update
        );
        for bad in [
            &["remove", "org.gimp.GIMP", "user", "--"][..],
            &["install", "gimp", "user", "--"],
            &["install", "org.gimp.GIMP", "root", "--"],
            &["install", "org.gimp.GIMP", "user", "x"],
            &["install", "org.gimp.GIMP", "user"],
        ] {
            assert_eq!(Request::parse(&a(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn app_ids() {
        for ok in [
            "org.gimp.GIMP",
            "com.valvesoftware.Steam",
            "io.github.foo_bar.App-1",
        ] {
            assert!(is_app_id(ok), "{ok}");
        }
        for bad in [
            "gimp",
            "org.gimp",
            "org..x",
            "-org.a.b",
            "org.a.b c",
            "org.a./b",
            "org.1a.b",
        ] {
            assert!(!is_app_id(bad), "{bad}");
        }
    }

    #[test]
    fn grants_from_the_consent_file() {
        let j = r#"{"grants":[
            {"app":"/home/ana/.local/share/flatpak/app/org.gimp.GIMP","capability":"flatpak:filesystems=home"},
            {"app":"/home/ana/.local/share/flatpak/app/org.gimp.GIMP","capability":"flatpak:filesystems=home"},
            {"app":"/var/lib/flatpak/app/org.gimp.GIMP","capability":"flatpak:shared=network"},
            {"app":"/usr/bin/grim","capability":"screen-capture"},
            {"app":"/x/app/org.gimp.GIMP","capability":"flatpak:bogus"}
        ]}"#;
        let g = parse_grants(j, Scope::User);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].app_id, "org.gimp.GIMP");
        assert_eq!(g[1].perm, p("shared=network"));
        assert!(parse_grants("nope", Scope::User).is_empty());
    }

    #[test]
    fn apps_group_grants_and_read_overrides() {
        use crate::system::fake::FakeHost;
        let h = FakeHost::linux();
        h.file(
                "/home/ana/.config/wolos/flatpak-consent.json",
                r#"{"grants":[{"app":"/home/ana/.local/share/flatpak/app/org.gimp.GIMP","capability":"flatpak:filesystems=home"},{"app":"/home/ana/.local/share/flatpak/app/org.gimp.GIMP","capability":"flatpak:shared=network"}]}"#,
            )
            .file(
                "/home/ana/.local/share/flatpak/overrides/org.gimp.GIMP",
                "[Context]\nshared=!network;\n",
            )
            .file(
                "/home/ana/.local/share/flatpak/app/org.gimp.GIMP/current/active/export/share/applications/org.gimp.GIMP.desktop",
                "[Desktop Entry]\nName=GIMP\n",
            )
            .file(
                "/etc/wolos/flatpak-consent.json",
                r#"{"grants":[{"app":"/var/lib/flatpak/app/com.valvesoftware.Steam","capability":"flatpak:devices=all"}]}"#,
            );
        let a = apps(&h, "/home/ana");
        assert_eq!(a.len(), 2);
        assert_eq!((a[0].name.as_str(), a[0].scope), ("GIMP", Scope::User));
        assert_eq!(a[0].icon, "org.gimp.GIMP");
        assert_eq!(
            a[0].perms
                .iter()
                .map(|(p, on)| (p.spelling(), *on))
                .collect::<Vec<_>>(),
            [
                ("filesystems=home".to_owned(), true),
                ("shared=network".to_owned(), false)
            ]
        );
        assert_eq!((a[1].name.as_str(), a[1].scope), ("Steam", Scope::System));
    }

    #[test]
    fn flathub_search_keeps_one_tile_per_app_and_prefers_native() {
        let out = "org.gimp.GIMP\tGIMP\tCreate images\tflathub\n\
                   org.kde.krita\tKrita\tDigital painting\tflathub\n\
                   org.kde.krita\tKrita\tDigital painting\tflathub\n\
                   com.example.Other\tOther\tFrom elsewhere\tsomeremote\n\
                   bad id\tX\tY\tflathub\n\
                   Application ID\tName\n";
        let native = |h: &Hit| native_guess(h) == "gimp";
        let v = parse_search(out, native);
        assert_eq!(
            v,
            [Hit {
                id: "org.kde.krita".into(),
                name: "Krita".into(),
                summary: "Digital painting".into()
            }]
        );
        assert_eq!(search_argv("-x"), None);
        assert_eq!(search_argv("  "), None);
        assert_eq!(
            search_argv("paint").unwrap().join(" "),
            "flatpak search --columns=application,name,description,remotes -- paint"
        );
    }

    #[test]
    fn store_argv_are_per_user() {
        assert_eq!(
            install_argv("org.kde.krita").join(" "),
            "flatpak install --user --noninteractive -y flathub org.kde.krita"
        );
        assert_eq!(
            remove_argv("org.kde.krita").join(" "),
            "flatpak uninstall --user --noninteractive -y org.kde.krita"
        );
        assert_eq!(
            update_argv().join(" "),
            "flatpak update --user --noninteractive -y"
        );
        assert_eq!(
            parse_ids("org.kde.krita\n\nApplication ID\norg.gimp.GIMP\n"),
            ["org.kde.krita", "org.gimp.GIMP"]
        );
    }

    #[test]
    fn names_from_the_exported_desktop_file() {
        let d = "[Desktop Entry]\nName=GNU Image Manipulation Program\nName[pt]=GIMP\n[Desktop Action new]\nName=New\n";
        assert_eq!(
            desktop_name(d).as_deref(),
            Some("GNU Image Manipulation Program")
        );
        assert_eq!(desktop_name("[Other]\nName=x\n"), None);
        assert_eq!(
            export_paths("/var/lib/flatpak", "org.gimp.GIMP").0,
            "/var/lib/flatpak/app/org.gimp.GIMP/current/active/export/share/applications/org.gimp.GIMP.desktop"
        );
    }
}
