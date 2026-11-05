// SPDX-License-Identifier: BSD-2-Clause
//! Web apps as apps (ADR 0031 W5): a web address becomes a launcher of its own, with a name and
//! an icon, that opens in a frameless browser window and shows up in Launchpad, Search and the
//! Dock like any installed app. This file is the pure part: checking the address, the launcher
//! id, the `.desktop` text, reading our launchers back and where the icon comes from. `wol
//! webapp add|remove|list` (crates/controller/src/verbs.rs) does the file writes and the icon
//! download. Invariant: a launcher is only ever built from an address that passed [`parse_url`],
//! so nothing a person typed reaches the `Exec=` line unchecked, and only files carrying
//! [`MARK_KEY`] are listed or removed.
use std::path::{Path, PathBuf};

/// Browser that opens web apps (`--app=` window: no tabs, no address bar): the launcher the
/// `atlas-browser` package installs (wolos-pkgbuilds atlas-browser/PKGBUILD), the one path
/// every browser caller shares ([`crate::browser::ATLAS`]). The package is named
/// `atlas-browser`; its program is `/usr/bin/atlas` (e2e D1: a launcher naming the package
/// opened nothing).
pub const BROWSER: &str = crate::browser::ATLAS.bin;
/// Profile directory the engine opens inside a user data directory (ours or the shared one).
const CHROMIUM_PROFILE: &str = "Default";
/// File name prefix of our launchers under `<data home>/applications`.
pub const PREFIX: &str = "wolos-webapp-";
/// Key in `[Desktop Entry]` that marks a launcher as ours; its value is the address.
pub const MARK_KEY: &str = "X-WolOS-WebApp";
/// Icons and own browser profiles live under `<data home>/<DATA_DIR>`.
pub const DATA_DIR: &str = "wolos/webapps";
/// Longest address accepted, in bytes (browsers cap near 2 MB; a launcher line should be short).
pub const MAX_URL: usize = 2048;
/// Longest name, in characters (the Dock tooltip and Launchpad label).
pub const MAX_NAME: usize = 64;
/// Largest icon download, in bytes: 1 MiB (an apple-touch-icon is about 20 KB).
pub const MAX_ICON_BYTES: u64 = 1 << 20;
/// Seconds an icon download may take before it is given up (the launcher works without it).
pub const ICON_TIMEOUT_SECS: u32 = 10;
/// Icon used when the site gives none.
pub const FALLBACK_ICON: &str = "web-browser";

/// Why a web app could not be made or found. `Display` is one sentence a person can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The address is not `https://host/...` (or `http://`), or has spaces or quotes.
    BadUrl,
    /// The name is empty, too long, or has no letter or digit to build an id from.
    BadName,
    /// A web app with this id already exists.
    Exists(String),
    /// No web app has this name or id.
    NotFound(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BadUrl => f.write_str(
                "Type the full web address, starting with https://, without spaces or quotes.",
            ),
            Error::BadName => f.write_str("Give the web app a short name with letters or digits."),
            Error::Exists(n) => write!(f, "A web app named \"{n}\" already exists."),
            Error::NotFound(n) => write!(f, "There is no web app named \"{n}\"."),
        }
    }
}

/// A checked web address: scheme `https` or `http`, the host, and the whole address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub url: String,
    pub https: bool,
    /// Lowercased host without port (`mail.example.com`).
    pub host: String,
}

/// Checks a web address a person typed. Returns [`Error::BadUrl`] unless it is `http(s)://`,
/// at most [`MAX_URL`] bytes, printable ASCII with no space, quote, backtick, backslash, `$` or
/// `%` outside an escape, and a host of letters, digits, `.` and `-` (plus an optional port).
/// A bare host (`example.com`) gets `https://` in front.
pub fn parse_url(input: &str) -> Result<Url, Error> {
    let s = input.trim();
    let s = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    if s.is_empty() || s.len() > MAX_URL {
        return Err(Error::BadUrl);
    }
    let ok_byte = |b: u8| b.is_ascii_graphic() && !b"\"'`\\$<>{}|^".contains(&b);
    if !s.bytes().all(ok_byte) || !percent_ok(&s) {
        return Err(Error::BadUrl);
    }
    let (https, rest) = if let Some(r) = s.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = s.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(Error::BadUrl);
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        // user:password@host hides the real host from the person reading the launcher.
        return Err(Error::BadUrl);
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|l| {
            !l.is_empty()
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.parse::<u16>().is_ok());
    if !host_ok || !port_ok {
        return Err(Error::BadUrl);
    }
    Ok(Url {
        host: host.to_ascii_lowercase(),
        url: s,
        https,
    })
}

/// Every `%` starts a two-hex-digit escape (so the `.desktop` `%` field codes cannot be forged).
fn percent_ok(s: &str) -> bool {
    let b = s.as_bytes();
    b.iter().enumerate().all(|(i, c)| {
        *c != b'%'
            || (b.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
    })
}

/// Most bytes of a [`slug`]: `org.wolos.webapp.` and the slug fill the 64 bytes a scope name keeps
/// (`shared::memsaver::APP_ID_MAX`); 48 made a 65-byte id that was cut (QA-WEB-26).
pub const SLUG_MAX: usize = 47;

/// Launcher id from a name: lowercase letters and digits joined by single `-`, at most
/// [`SLUG_MAX`] bytes (`"My Mail!"` -> `my-mail`). None when no letter or digit is left.
pub fn slug(name: &str) -> Option<String> {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if out.len() >= SLUG_MAX {
                break;
            }
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    (!out.is_empty()).then_some(out)
}

/// Checks a display name: trimmed, 1..=[`MAX_NAME`] characters, no control characters, and
/// [`slug`]-able. Returns the trimmed name, or [`Error::BadName`].
pub fn check_name(name: &str) -> Result<String, Error> {
    let n = name.trim();
    let ok = !n.is_empty() && n.chars().count() <= MAX_NAME && !n.chars().any(char::is_control);
    if ok && slug(n).is_some() {
        Ok(n.to_string())
    } else {
        Err(Error::BadName)
    }
}

/// Which browser profile a web app uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// The browser's normal profile: signed in wherever the browser is.
    Shared,
    /// A profile of its own under `<data home>/wolos/webapps/profiles/<id>`: separate cookies.
    Own,
}

/// One web app launcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebApp {
    /// Launcher id ([`slug`] of the name).
    pub id: String,
    pub name: String,
    pub url: String,
    /// Icon file path or icon theme name.
    pub icon: String,
    pub profile: Profile,
}

impl WebApp {
    /// A web app from a checked name and address; icon falls back to [`FALLBACK_ICON`].
    /// Returns [`Error::BadName`] or [`Error::BadUrl`] when either does not check.
    pub fn new(name: &str, url: &str, profile: Profile) -> Result<WebApp, Error> {
        let name = check_name(name)?;
        let url = parse_url(url)?;
        Ok(WebApp {
            id: slug(&name).ok_or(Error::BadName)?,
            name,
            url: url.url,
            icon: FALLBACK_ICON.into(),
            profile,
        })
    }
}

/// `<data home>/applications/wolos-webapp-<id>.desktop`.
pub fn desktop_path(data_home: &Path, id: &str) -> PathBuf {
    data_home
        .join("applications")
        .join(format!("{PREFIX}{id}.desktop"))
}

/// `<data home>/wolos/webapps/icons/<id>.<ext>`.
pub fn icon_path(data_home: &Path, id: &str, ext: &str) -> PathBuf {
    data_home
        .join(DATA_DIR)
        .join("icons")
        .join(format!("{id}.{ext}"))
}

/// `<data home>/wolos/webapps/profiles/<id>`.
pub fn profile_dir(data_home: &Path, id: &str) -> PathBuf {
    data_home.join(DATA_DIR).join("profiles").join(id)
}

/// Window class (Wayland app-id) the browser gives an `--app` window, so the Dock and the menu
/// bar find the launcher (`StartupWMClass`). The engine ignores `--class` for app windows on
/// Wayland and names them after the address: `chrome-<host>_<path, / as _>-<profile>`, without
/// port, query or fragment (measured on the 0.20.2.1 browser build: `https://example.org/` gives
/// `chrome-example.org__-Default`, `https://Mail.Example.org:8443/a/b.c?x=1#y` gives
/// `chrome-mail.example.org__a_b.c-Default`).
pub fn wm_class(app: &WebApp) -> String {
    let Ok(u) = parse_url(&app.url) else {
        return format!("{PREFIX}{}", app.id);
    };
    let rest = u.url.split_once("://").map_or("", |(_, r)| r);
    let path = rest
        .find(['/', '?', '#'])
        .map_or("", |i| &rest[i..])
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    let path = if path.is_empty() { "/" } else { path };
    format!(
        "chrome-{}_{}-{CHROMIUM_PROFILE}",
        u.host,
        path.replace('/', "_")
    )
}

/// Escapes one `Exec=` argument (freedesktop Desktop Entry spec, "The Exec key"): quoted, with
/// `"`, `` ` ``, `$` and `\` escaped, and `%` doubled so it is never a field code.
fn exec_arg(a: &str) -> String {
    let mut s = String::from("\"");
    for c in a.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                s.push_str("\\\\");
                s.push(c);
            }
            '%' => s.push_str("%%"),
            _ => s.push(c),
        }
    }
    s.push('"');
    s
}

/// An icon value a launcher may carry (audit C6-1): an absolute path or an icon theme name with
/// no control characters (a newline would start a new key) and no backslash, at most 4096 bytes.
pub fn check_icon(icon: &str) -> bool {
    !icon.is_empty() && icon.len() <= 4096 && !icon.chars().any(|c| c.is_control() || c == '\\')
}

/// Escapes one string value (freedesktop Desktop Entry spec, "Possible value types"): backslash,
/// newline, tab and return become `\\`, `\n`, `\t`, `\r`; any other control character is
/// dropped. The value can never span two lines.
fn value(v: &str) -> String {
    let mut s = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\t' => s.push_str("\\t"),
            '\r' => s.push_str("\\r"),
            c if c.is_control() => {}
            c => s.push(c),
        }
    }
    s
}

/// Reverses [`value`] for the keys we read back.
fn unvalue(v: &str) -> String {
    let mut s = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            s.push(c);
            continue;
        }
        match it.next() {
            Some('n') => s.push('\n'),
            Some('t') => s.push('\t'),
            Some('r') => s.push('\r'),
            Some('s') => s.push(' '),
            Some(o) => s.push(o),
            None => s.push('\\'),
        }
    }
    s
}

/// The browser argv that opens `app` (data home for an own profile).
pub fn launch_argv(app: &WebApp, data_home: &Path) -> Vec<String> {
    let mut v = vec![BROWSER.to_string(), format!("--app={}", app.url)];
    if app.profile == Profile::Own {
        v.push(format!(
            "--user-data-dir={}",
            profile_dir(data_home, &app.id).display()
        ));
    }
    v
}

/// The `.desktop` text for `app`. Every value is escaped ([`value`]) so it stays on one line,
/// whatever the caller put in it (audit C6-1); the name has no control characters
/// ([`check_name`]), the address no spaces ([`parse_url`]) and the icon passed [`check_icon`].
pub fn desktop_entry(app: &WebApp, data_home: &Path) -> String {
    let exec: Vec<String> = launch_argv(app, data_home)
        .iter()
        .map(|a| exec_arg(a))
        .collect();
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName={name}\nComment={url}\n\
         Exec={exec}\nIcon={icon}\nTerminal=false\nCategories=Network;WebBrowser;\n\
         StartupWMClass={class}\n{MARK_KEY}={url}\n{MARK_KEY}-Profile={profile}\n",
        name = value(&app.name),
        url = value(&app.url),
        exec = exec.join(" "),
        icon = value(&app.icon),
        class = value(&wm_class(app)),
        profile = match app.profile {
            Profile::Shared => "shared",
            Profile::Own => "own",
        },
    )
}

/// Reads one of our launchers back (`id` from its file name). None when the text is not a
/// launcher we wrote (no [`MARK_KEY`], or a name or address that no longer checks).
pub fn parse_entry(id: &str, text: &str) -> Option<WebApp> {
    // The one `[Desktop Entry]` reader (trimmed, first key wins, empty = absent).
    let e = crate::desktop::Entry::parse(text);
    let (name, url, icon) = (e.name(), e.get(MARK_KEY), e.icon());
    let own = e.get(&format!("{MARK_KEY}-Profile")) == Some("own");
    let name = check_name(&unvalue(name?)).ok()?;
    let url = parse_url(&unvalue(url?)).ok()?;
    let icon = icon.map(unvalue).filter(|i| check_icon(i));
    Some(WebApp {
        id: id.to_string(),
        name,
        url: url.url,
        icon: icon.unwrap_or_else(|| FALLBACK_ICON.to_string()),
        profile: if own { Profile::Own } else { Profile::Shared },
    })
}

/// Launcher id from a file name in `applications/` (`wolos-webapp-mail.desktop` -> `mail`).
pub fn id_of_file(file_name: &str) -> Option<&str> {
    let id = file_name.strip_prefix(PREFIX)?.strip_suffix(".desktop")?;
    (slug(id).as_deref() == Some(id)).then_some(id)
}

/// Picks the web app a person meant: by id first, then by name (case-insensitive).
pub fn find<'a>(apps: &'a [WebApp], what: &str) -> Option<&'a WebApp> {
    let w = what.trim();
    apps.iter()
        .find(|a| a.id == w)
        .or_else(|| apps.iter().find(|a| a.name.eq_ignore_ascii_case(w)))
        .or_else(|| {
            let s = slug(w)?;
            apps.iter().find(|a| a.id == s)
        })
}

/// Where to look for a site's icon, best first: the touch icon (large PNG), then the favicon.
/// Each entry is (address, file extension). Only for https sites: an icon over plain http could
/// be swapped on the way.
pub fn icon_candidates(url: &Url) -> Vec<(String, &'static str)> {
    if !url.https {
        return Vec::new();
    }
    vec![
        (format!("https://{}/apple-touch-icon.png", url.host), "png"),
        (format!("https://{}/favicon.ico", url.host), "ico"),
    ]
}

/// `curl` argv that downloads one icon to `out`: https only (also on redirects), at most
/// [`MAX_ICON_BYTES`], [`ICON_TIMEOUT_SECS`], no config file, `--` before the address.
pub fn icon_fetch_argv(icon_url: &str, out: &Path) -> Vec<String> {
    vec![
        "curl".into(),
        "--disable".into(),
        "--fail".into(),
        "--silent".into(),
        "--location".into(),
        "--proto".into(),
        "=https".into(),
        "--proto-redir".into(),
        "=https".into(),
        "--max-time".into(),
        ICON_TIMEOUT_SECS.to_string(),
        "--max-filesize".into(),
        MAX_ICON_BYTES.to_string(),
        "--output".into(),
        out.display().to_string(),
        "--".into(),
        icon_url.into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C6-1: an icon with a newline cannot add a key; escaped values read back unchanged.
    #[test]
    fn desktop_entry_values_never_span_two_lines() {
        let mut app = WebApp::new("Mail", "https://mail.example.com", Profile::Shared).unwrap();
        app.icon = "/x.png\nExec=evil\r\u{1}".into();
        let text = desktop_entry(&app, Path::new("/data"));
        assert_eq!(text.lines().filter(|l| l.starts_with("Exec=")).count(), 1);
        // The escaped text still holds the letters "Exec=evil" inside the Icon value; what must
        // never happen is a line (a key) that starts with them.
        assert!(!text.lines().any(|l| l.starts_with("Exec=evil")));
        assert_eq!(text.lines().filter(|l| l.starts_with("Icon=")).count(), 1);
        assert!(text.contains("\nIcon=/x.png\\nExec=evil\\r\n"));
        assert!(!check_icon("/x.png\nExec=evil") && !check_icon("a\\b") && !check_icon(""));
        assert!(check_icon("/usr/share/icons/a b.png") && check_icon("mail-send"));
        let back = parse_entry("mail", &text).unwrap();
        assert_eq!(
            back.icon, FALLBACK_ICON,
            "a read-back icon that fails the check is dropped"
        );
        app.icon = "/home/me/my icon.png".into();
        let back = parse_entry("mail", &desktop_entry(&app, Path::new("/data"))).unwrap();
        assert_eq!(back.icon, app.icon);
        assert_eq!(unvalue(&value("a\\b\tc")), "a\\b\tc");
    }

    #[test]
    fn addresses_are_checked_before_they_reach_a_launcher() {
        let u = parse_url("https://Mail.Example.com/inbox?x=1#y").unwrap();
        assert_eq!((u.host.as_str(), u.https), ("mail.example.com", true));
        assert_eq!(parse_url("example.com").unwrap().url, "https://example.com");
        assert_eq!(parse_url("  http://a.b:8080/ ").unwrap().host, "a.b");
        assert!(parse_url("http://a.b:8080/").is_ok_and(|u| !u.https));
        for bad in [
            "",
            "ftp://a.b/",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://a.b/ x",
            "https://a.b/\"; rm -rf ~",
            "https://a.b/$(id)",
            "https://a.b/`id`",
            "https://a.b/\\x",
            "https://a.b/%u",
            "https://a.b/%",
            "https://user:pw@a.b/",
            "https://localhost/",
            "https://-a.b/",
            "https://a.b:99999/",
            "https://a.b:/",
            "https://a.b/\tx",
            "https://ä.b/",
        ] {
            assert_eq!(parse_url(bad), Err(Error::BadUrl), "{bad:?}");
        }
        assert!(parse_url(&format!("https://a.b/{}", "x".repeat(MAX_URL))).is_err());
        assert!(parse_url("https://a.b/%20ok").is_ok());
    }

    #[test]
    fn ids_and_names() {
        assert_eq!(slug("My Mail!").as_deref(), Some("my-mail"));
        assert_eq!(
            slug("  --YouTube  Music--").as_deref(),
            Some("youtube-music")
        );
        assert_eq!(slug("日本"), None);
        // 47: with the host's `org.wolos.webapp.` (17 bytes) the id fills the 64 bytes a scope
        // name keeps (`shared::memsaver::APP_ID_MAX`), never more (QA-WEB-26).
        assert_eq!(slug(&"a".repeat(80)).map(|s| s.len()), Some(SLUG_MAX));
        assert_eq!(
            "org.wolos.webapp.".len() + SLUG_MAX,
            shared::memsaver::APP_ID_MAX
        );
        assert_eq!(check_name(" Mail "), Ok("Mail".into()));
        for bad in ["", "   ", "!!!", "a\nb", &"m".repeat(MAX_NAME + 1)] {
            assert_eq!(check_name(bad), Err(Error::BadName), "{bad:?}");
        }
    }

    #[test]
    fn the_launcher_opens_a_frameless_window_and_reads_back() {
        let home = Path::new("/home/a/.local/share");
        let mut app =
            WebApp::new("Mail", "https://mail.example.com/?a=1%20b", Profile::Own).unwrap();
        app.icon = icon_path(home, &app.id, "png").display().to_string();
        let text = desktop_entry(&app, home);
        assert!(text.starts_with("[Desktop Entry]\nType=Application\n"));
        assert!(text.contains(
            "\nExec=\"/usr/bin/atlas\" \"--app=https://mail.example.com/?a=1%%20b\" \
             \"--user-data-dir=/home/a/.local/share/wolos/webapps/profiles/mail\"\n"
        ));
        assert!(text.contains("\nStartupWMClass=chrome-mail.example.com__-Default\n"));
        assert!(text.contains("\nIcon=/home/a/.local/share/wolos/webapps/icons/mail.png\n"));
        assert_eq!(parse_entry("mail", &text), Some(app.clone()));
        let shared = WebApp {
            profile: Profile::Shared,
            ..app
        };
        let t2 = desktop_entry(&shared, home);
        assert!(!t2.contains("--user-data-dir"));
        assert_eq!(parse_entry("mail", &t2), Some(shared));
        // Somebody else's launcher, or one edited to carry a bad address, is not ours.
        assert_eq!(parse_entry("x", "[Desktop Entry]\nName=X\nExec=x\n"), None);
        assert_eq!(
            parse_entry("x", "[Desktop Entry]\nName=X\nX-WolOS-WebApp=file:///etc\n"),
            None
        );
    }

    /// e2e D1: the window class the browser really gives an `--app` window (measured in the VM), so
    /// the Dock and the menu bar match the launcher instead of showing `chrome-...`.
    #[test]
    fn the_window_class_is_the_one_the_engine_gives_app_windows() {
        let class = |url: &str| wm_class(&WebApp::new("Mail", url, Profile::Own).unwrap());
        assert_eq!(class("https://example.org"), "chrome-example.org__-Default");
        assert_eq!(
            class("https://example.org/"),
            "chrome-example.org__-Default"
        );
        assert_eq!(
            class("https://Mail.Example.org:8443/a/b.c?x=1#y"),
            "chrome-mail.example.org__a_b.c-Default"
        );
        assert_eq!(class("example.org?q=1"), "chrome-example.org__-Default");
        // The browser is the program the package installs, never the package name.
        assert_eq!(BROWSER, "/usr/bin/atlas");
        assert!(
            !launch_argv(
                &WebApp::new("M", "example.org", Profile::Shared).unwrap(),
                Path::new("/d")
            )
            .iter()
            .any(|a| a.starts_with("--class"))
        );
    }

    #[test]
    fn a_launcher_reads_back_like_every_desktop_entry() {
        // wol_core::desktop::Entry's rules: trimmed, the first key wins, other groups skipped.
        let text = "[Desktop Entry]\nName = Mail \nName=Other\nX-WolOS-WebApp= https://mail.example.com/ \n\
                    X-WolOS-WebApp-Profile=own\n[Desktop Action new]\nName=New\nIcon=x\n";
        let app = parse_entry("mail", text).unwrap();
        assert_eq!(
            (app.name.as_str(), app.url.as_str(), app.profile),
            ("Mail", "https://mail.example.com/", Profile::Own)
        );
        assert_eq!(
            app.icon, FALLBACK_ICON,
            "an action's Icon is not the launcher's"
        );
    }

    #[test]
    fn exec_arguments_are_quoted() {
        assert_eq!(exec_arg("a b"), "\"a b\"");
        assert_eq!(exec_arg("%u"), "\"%%u\"");
        assert_eq!(exec_arg("a\"b"), "\"a\\\\\"b\"");
        assert_eq!(exec_arg("$x"), "\"\\\\$x\"");
    }

    #[test]
    fn files_and_lookup() {
        let d = Path::new("/d");
        assert_eq!(
            desktop_path(d, "mail"),
            PathBuf::from("/d/applications/wolos-webapp-mail.desktop")
        );
        assert_eq!(id_of_file("wolos-webapp-mail.desktop"), Some("mail"));
        assert_eq!(id_of_file("wolos-webapp-../x.desktop"), None);
        assert_eq!(id_of_file("org.gnome.Maps.desktop"), None);
        let apps = vec![
            WebApp::new("Mail", "https://m.example.com", Profile::Shared).unwrap(),
            WebApp::new(
                "YouTube Music",
                "https://music.youtube.com",
                Profile::Shared,
            )
            .unwrap(),
        ];
        assert_eq!(find(&apps, "mail").map(|a| a.name.as_str()), Some("Mail"));
        assert_eq!(
            find(&apps, "youtube music").map(|a| a.id.as_str()),
            Some("youtube-music")
        );
        assert_eq!(
            find(&apps, "YouTube-Music!").map(|a| a.id.as_str()),
            Some("youtube-music")
        );
        assert!(find(&apps, "maps").is_none());
    }

    #[test]
    fn icons_come_from_the_site_over_https_only() {
        let u = parse_url("https://mail.example.com/inbox").unwrap();
        let c = icon_candidates(&u);
        assert_eq!(c[0].0, "https://mail.example.com/apple-touch-icon.png");
        assert_eq!(c[1], ("https://mail.example.com/favicon.ico".into(), "ico"));
        assert!(icon_candidates(&parse_url("http://a.b").unwrap()).is_empty());
        let a = icon_fetch_argv(&c[0].0, Path::new("/tmp/i.png"));
        assert_eq!(a[0], "curl");
        assert_eq!(a[1], "--disable", "no ~/.curlrc");
        let dd = a.iter().position(|x| x == "--").unwrap();
        assert_eq!(a.get(dd + 1), Some(&c[0].0));
        assert!(a.windows(2).any(|w| w == ["--proto-redir", "=https"]));
        assert!(a.windows(2).any(|w| w == ["--max-filesize", "1048576"]));
    }

    #[test]
    fn errors_are_sentences() {
        for e in [
            Error::BadUrl,
            Error::BadName,
            Error::Exists("Mail".into()),
            Error::NotFound("Mail".into()),
        ] {
            let s = e.to_string();
            assert!(s.ends_with('.') && s.starts_with(char::is_uppercase), "{s}");
        }
    }
}
