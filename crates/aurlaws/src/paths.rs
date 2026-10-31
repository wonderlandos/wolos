// SPDX-License-Identifier: GPL-2.0-or-later
//! What a path means to a law.

const CREDENTIALS: &[(&str, &str)] = &[
    ("/.ssh", "SSH keys"),
    ("/.gnupg", "GPG keys"),
    ("/.aws/", "AWS credentials"),
    ("/.gcloud", "cloud credentials"),
    ("/.config/gcloud", "cloud credentials"),
    ("/.kube/config", "cluster credentials"),
    ("/.docker/config.json", "registry credentials"),
    ("/.netrc", "login credentials"),
    ("/.git-credentials", "git credentials"),
    ("/.config/gh/hosts.yml", "git credentials"),
    ("/.npmrc", "registry credentials"),
    ("/.pypirc", "registry credentials"),
    ("/.cargo/credentials", "registry credentials"),
    ("/.password-store", "a password store"),
    ("/.local/share/keyrings", "a keyring"),
    ("/.mozilla/firefox", "a browser profile"),
    ("/.librewolf", "a browser profile"),
    ("/.thunderbird", "a mail profile"),
    ("/.config/google-chrome", "a browser profile"),
    ("/.config/chromium", "a browser profile"),
    ("/.config/BraveSoftware", "a browser profile"),
    ("/.config/microsoft-edge", "a browser profile"),
    ("/.config/vivaldi", "a browser profile"),
    ("/.config/opera", "a browser profile"),
    ("/.config/discord", "a chat session"),
    ("/.config/Signal", "a chat session"),
    ("/.local/share/TelegramDesktop", "a chat session"),
    ("/.bitcoin", "a crypto wallet"),
    ("/.electrum", "a crypto wallet"),
    ("/etc/shadow", "system passwords"),
    ("/etc/gshadow", "system passwords"),
    ("/etc/sudoers", "system privileges"),
];

/// What sits inside these starts by itself. The folders alone do nothing.
const PERSISTENCE_DIRS: &[&str] = &[
    "/etc/cron.d",
    "/etc/cron.hourly",
    "/etc/cron.daily",
    "/etc/cron.weekly",
    "/etc/cron.monthly",
    "/var/spool/cron",
    "/var/spool/at",
    "/etc/systemd/system",
    "/etc/systemd/user",
    "/usr/lib/systemd/system",
    "/usr/lib/systemd/user",
    "/var/lib/systemd/linger",
    "/etc/xdg/autostart",
    "/etc/profile.d",
    "/etc/init.d",
    "/etc/pacman.d/hooks",
    "/usr/share/libalpm/hooks",
];

const PERSISTENCE_FILES: &[&str] = &[
    "/etc/crontab",
    "/etc/anacrontab",
    "/etc/profile",
    "/etc/bash.bashrc",
    "/etc/rc.local",
    "/etc/ld.so.preload",
];

const HOME_PERSISTENCE: &[&str] = &[
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".profile",
    ".zshrc",
    ".zprofile",
    ".zshenv",
    ".xprofile",
    ".xinitrc",
    ".config/fish/config.fish",
    ".config/autostart",
    ".config/systemd/user",
    ".ssh/authorized_keys",
];

const TOOL_CACHES: &[&str] = &[
    ".cache",
    ".cargo",
    ".rustup",
    ".npm",
    ".node-gyp",
    ".yarn",
    ".pnpm-store",
    ".gradle",
    ".m2",
    ".nuget",
    ".dotnet",
    ".local/share/pnpm",
    "go",
];

/// Paths inside the build's own folders are its own files, whatever they are called.
pub fn inside_build(p: &str) -> bool {
    ["$srcdir", "$pkgdir", "$startdir"]
        .iter()
        .any(|r| p == *r || p.strip_prefix(r).is_some_and(|x| x.starts_with('/')))
}

fn is_or_under(p: &str, dir: &str) -> bool {
    p.strip_prefix(dir)
        .is_some_and(|r| r.is_empty() || r.starts_with('/'))
}

/// `$pkgdir/etc/x` is `/etc/x` once installed.
pub fn installed(p: &str) -> &str {
    p.strip_prefix("$pkgdir").unwrap_or(p)
}

/// What secret lives at this path, if any.
pub fn credential(p: &str) -> Option<&'static str> {
    if inside_build(p) {
        return None;
    }
    CREDENTIALS
        .iter()
        .find(|(c, _)| {
            if c.ends_with('/') {
                p.contains(c)
            } else {
                p.contains(c)
                    && !p[p.find(c).unwrap_or(0) + c.len()..]
                        .starts_with(|x: char| x.is_alphanumeric())
            }
        })
        .map(|x| x.1)
}

/// Does something here start again by itself after a reboot or login?
pub fn persistence(p: &str) -> bool {
    let i = installed(p);
    let in_dir = PERSISTENCE_DIRS.iter().any(|d| {
        i.strip_prefix(d)
            .is_some_and(|r| r.len() > 1 && r.starts_with('/'))
    });
    if in_dir || PERSISTENCE_FILES.contains(&i) {
        return true;
    }
    let home = i
        .strip_prefix("$HOME/")
        .or_else(|| i.strip_prefix("/root/"))
        .or_else(|| {
            i.strip_prefix("/home/")
                .and_then(|r| r.split_once('/').map(|x| x.1))
        });
    home.is_some_and(|h| HOME_PERSISTENCE.iter().any(|d| is_or_under(h, d)))
}

/// A file that does nothing until something enables it, while it sits in a package's own list.
pub fn inert_in_package(p: &str) -> bool {
    p.starts_with("$pkgdir") && installed(p).contains("/systemd/")
}

/// A build may scribble here without leaving its folders in any way that matters.
fn scratch(p: &str) -> bool {
    p.starts_with("pipe:")
        || ["/tmp", "/var/tmp", "/dev", "/proc", "/run/user"]
            .iter()
            .any(|d| is_or_under(p, d))
}

fn tool_cache(p: &str) -> bool {
    p.strip_prefix("$HOME/")
        .is_some_and(|h| TOOL_CACHES.iter().any(|d| is_or_under(h, d)))
}

/// A write a build has no business making. Paths we could not resolve are not judged here.
pub fn outside_package(p: &str) -> bool {
    let known = p.starts_with('/') || is_or_under(p, "$HOME");
    known && !inside_build(p) && !scratch(p) && !tool_cache(p)
}

/// Not a real file for a scriptlet law.
pub fn harmless(p: &str) -> bool {
    scratch(p)
}
